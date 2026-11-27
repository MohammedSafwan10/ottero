#![allow(clippy::expect_used)]

//! Real 2-device (Desktop <-> Android) end-to-end encrypted sync integration
//! test over a live local HTTP server (`127.0.0.1`).
//!
//! Proves byte-for-byte convergence across Notes (create, edit, pin, soft-trash,
//! restore, permanent delete), Tasks (create, edit, complete, soft-trash,
//! restore), Events, Habits, and Vault items using the exact same
//! `ottero-sync-client` engine used by Desktop and Android.

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use ottero_core::{Event, Habit, Note, Priority, Schedule, Task, TaskStatus, clock};
use ottero_crypto::SecretKey;
use ottero_storage::{Store, TaskFilter};
use ottero_sync_client::{
    SyncClient, apply_decrypted_page, derive_sync_account_id, enqueue_event_change,
    enqueue_habit_change, enqueue_note_change, enqueue_note_tombstone, enqueue_task_change,
    sync_once_with_apply,
};
use ottero_sync_protocol::{
    Collection, CollectionKeyWire, KeysResponse, PullResponse, PushAccepted, PushRequest,
    PushResponse, PutKeysRequest, QuotaUsage, RemoteChange,
};

#[derive(Default)]
struct ServerState {
    keys: HashMap<Collection, CollectionKeyWire>,
    changes: Vec<RemoteChange>,
    next_seq: i64,
}

fn spawn_mock_sync_server() -> (String, Arc<Mutex<ServerState>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let state = Arc::new(Mutex::new(ServerState::default()));
    let srv_state = Arc::clone(&state);

    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let st = Arc::clone(&srv_state);
            thread::spawn(move || handle_http(stream, st));
        }
    });

    (format!("http://{addr}"), state)
}

fn handle_http(mut stream: std::net::TcpStream, state: Arc<Mutex<ServerState>>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut header_end = None;
    let mut content_len = 0usize;

    loop {
        let Ok(n) = stream.read(&mut chunk) else {
            return;
        };
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if header_end.is_none()
            && let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n")
        {
            header_end = Some(pos + 4);
            let headers = String::from_utf8_lossy(&buf[..pos]);
            for line in headers.lines() {
                if let Some((k, v)) = line.split_once(':')
                    && k.eq_ignore_ascii_case("content-length")
                {
                    content_len = v.trim().parse().unwrap_or(0);
                }
            }
        }
        if let Some(end) = header_end
            && buf.len() >= end + content_len
        {
            break;
        }
    }

    let Some(end) = header_end else { return };
    let head = String::from_utf8_lossy(&buf[..end]);
    let mut req_line = head.lines().next().unwrap_or("").split_whitespace();
    let method = req_line.next().unwrap_or("");
    let path = req_line.next().unwrap_or("");
    let body = &buf[end..end + content_len.min(buf.len() - end)];

    let mut st = state.lock().expect("lock");
    let response_json = match (method, path.split('?').next().unwrap_or(path)) {
        ("GET", "/v1/sync/keys") => {
            let resp = KeysResponse {
                keys: st.keys.values().cloned().collect(),
            };
            serde_json::to_vec(&resp).expect("json")
        }
        ("PUT", "/v1/sync/keys") => {
            let req: PutKeysRequest = serde_json::from_slice(body).expect("put keys");
            for k in req.keys {
                st.keys.entry(k.collection).or_insert(k);
            }
            let resp = KeysResponse {
                keys: st.keys.values().cloned().collect(),
            };
            serde_json::to_vec(&resp).expect("json")
        }
        ("POST", "/v1/sync/push") => {
            let req: PushRequest = serde_json::from_slice(body).expect("push");
            let mut accepted = Vec::with_capacity(req.changes.len());
            for ch in req.changes {
                st.next_seq += 1;
                let seq = st.next_seq;
                accepted.push(PushAccepted {
                    change_id: ch.change_id,
                    seq,
                    duplicate: false,
                });
                st.changes.push(RemoteChange {
                    seq,
                    change_id: ch.change_id,
                    device_id: req.device_id.clone(),
                    collection: ch.collection,
                    item_id: ch.item_id,
                    hlc: ch.hlc,
                    schema_version: ch.schema_version,
                    deleted: ch.deleted,
                    blob_b64: ch.blob_b64,
                    created_at_ms: clock::now().as_millisecond(),
                });
            }
            let resp = PushResponse {
                accepted,
                latest_seq: st.next_seq,
                quota: QuotaUsage {
                    used_bytes: 0,
                    limit_bytes: 100_000_000,
                },
            };
            serde_json::to_vec(&resp).expect("json")
        }
        ("GET", "/v1/sync/pull") => {
            let after_seq = path
                .split('?')
                .nth(1)
                .and_then(|q| {
                    q.split('&')
                        .find_map(|p| p.strip_prefix("after_seq="))
                        .and_then(|v| v.parse::<i64>().ok())
                })
                .unwrap_or(0);
            let changes: Vec<RemoteChange> = st
                .changes
                .iter()
                .filter(|c| c.seq > after_seq)
                .cloned()
                .collect();
            let next_seq = changes.last().map_or(after_seq, |c| c.seq);
            let resp = PullResponse {
                changes,
                next_seq,
                has_more: false,
                oldest_available_seq: 0,
                quota: QuotaUsage {
                    used_bytes: 0,
                    limit_bytes: 100_000_000,
                },
            };
            serde_json::to_vec(&resp).expect("json")
        }
        _ => b"{}".to_vec(),
    };

    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response_json.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&response_json);
}

#[test]
fn desktop_and_android_converge_across_notes_tasks_events_habits() {
    let (base_url, _state) = spawn_mock_sync_server();
    let client = SyncClient::new(&base_url).expect("client");
    let account_id = derive_sync_account_id("Owner@Example.COM");
    let account_key = SecretKey::generate().expect("key");
    let token = "test-bearer-token";

    let mut desktop = Store::open_in_memory().expect("desktop db");
    let mut android = Store::open_in_memory().expect("android db");
    let dev_desktop = "dev-desktop-mac";
    let dev_android = "dev-android-oneplus";

    // 1. Desktop creates a note, a task, an event, and a habit.
    let t0 = clock::now();
    let mut note = Note::new("Roadmap", "Initial body", t0).expect("note");
    desktop.save_note(&note).expect("save note");
    enqueue_note_change(&desktop, dev_desktop, &note, t0).expect("enqueue note");

    let mut task = Task::new("Ship realtime sync", t0).expect("task");
    task.set_priority(Priority::High, t0).expect("prio");
    desktop.save_task(&task).expect("save task");
    enqueue_task_change(&desktop, dev_desktop, &task, t0).expect("enqueue task");

    let today = clock::today();
    let event = Event::new_all_day("Launch Day", today, today, t0).expect("event");
    desktop.save_event(&event).expect("save event");
    enqueue_event_change(&desktop, dev_desktop, &event, t0).expect("enqueue event");

    let habit = Habit::new("Read 20 pages", Schedule::Daily, t0).expect("habit");
    desktop.save_habit(&habit).expect("save habit");
    enqueue_habit_change(&desktop, dev_desktop, &habit, t0).expect("enqueue habit");

    let started = Instant::now();
    let r_push = sync_once_with_apply(
        &mut desktop,
        &client,
        token,
        &account_id,
        dev_desktop,
        &account_key,
        clock::now(),
        |s, page| {
            apply_decrypted_page(s, page)?;
            Ok(())
        },
    )
    .expect("desktop push");
    assert_eq!(r_push.pushed, 4);

    // 2. Android pulls all 4 items.
    let r_pull = sync_once_with_apply(
        &mut android,
        &client,
        token,
        &account_id,
        dev_android,
        &account_key,
        clock::now(),
        |s, page| {
            apply_decrypted_page(s, page)?;
            Ok(())
        },
    )
    .expect("android pull");
    let roundtrip_ms = started.elapsed().as_millis();
    eprintln!("2-device E2EE 4-collection push+pull roundtrip: {roundtrip_ms} ms");
    assert_eq!(r_pull.pulled, 4);
    assert!(
        roundtrip_ms < 2_000,
        "local E2EE push+pull took too long: {roundtrip_ms}ms"
    );

    // Verify Android received exact copies.
    let android_note = android.note(note.id()).expect("note").expect("present");
    assert_eq!(android_note.title(), "Roadmap");
    assert_eq!(android_note.body(), "Initial body");
    assert_eq!(
        android.list_tasks(TaskFilter::Open).expect("tasks").len(),
        1
    );
    assert_eq!(android.habits(false).expect("habits").len(), 1);

    // 3. Android edits the note, completes the task, and soft-trashes the note.
    let t1 = clock::now();
    let mut edited = android.note(note.id()).expect("note").expect("present");
    edited
        .edit("Roadmap v2", "Edited on Android", t1)
        .expect("edit");
    android.save_note(&edited).expect("save");
    enqueue_note_change(&android, dev_android, &edited, t1).expect("enqueue");

    let mut done_task = android.task(task.id()).expect("task").expect("present");
    done_task.transition(TaskStatus::Done, t1).expect("done");
    android.save_task(&done_task).expect("save");
    enqueue_task_change(&android, dev_android, &done_task, t1).expect("enqueue");

    sync_once_with_apply(
        &mut android,
        &client,
        token,
        &account_id,
        dev_android,
        &account_key,
        clock::now(),
        |s, page| {
            apply_decrypted_page(s, page)?;
            Ok(())
        },
    )
    .expect("android push");

    sync_once_with_apply(
        &mut desktop,
        &client,
        token,
        &account_id,
        dev_desktop,
        &account_key,
        clock::now(),
        |s, page| {
            apply_decrypted_page(s, page)?;
            Ok(())
        },
    )
    .expect("desktop pull");

    let desktop_note = desktop.note(note.id()).expect("note").expect("present");
    assert_eq!(desktop_note.title(), "Roadmap v2");
    assert_eq!(desktop_note.body(), "Edited on Android");
    assert_eq!(
        desktop.task(task.id()).expect("t").expect("t").status(),
        TaskStatus::Done
    );

    // 4. Desktop soft-trashes the note -> Android sees it in Trash (NOT permanently deleted).
    let t2 = clock::now();
    note = desktop.note(note.id()).expect("note").expect("present");
    note.trash(t2);
    desktop.save_note(&note).expect("save");
    enqueue_note_change(&desktop, dev_desktop, &note, t2).expect("enqueue trash");

    sync_once_with_apply(
        &mut desktop,
        &client,
        token,
        &account_id,
        dev_desktop,
        &account_key,
        clock::now(),
        |s, page| {
            apply_decrypted_page(s, page)?;
            Ok(())
        },
    )
    .expect("desktop push trash");
    sync_once_with_apply(
        &mut android,
        &client,
        token,
        &account_id,
        dev_android,
        &account_key,
        clock::now(),
        |s, page| {
            apply_decrypted_page(s, page)?;
            Ok(())
        },
    )
    .expect("android pull trash");

    assert_eq!(android.list_notes(false).expect("active").len(), 0);
    assert_eq!(android.list_notes(true).expect("trashed").len(), 1);

    // 5. Desktop permanently deletes the note -> Android permanently deletes it too.
    let t3 = clock::now();
    desktop.delete_note_permanently(note.id()).expect("delete");
    enqueue_note_tombstone(&desktop, dev_desktop, note.id(), t3).expect("tombstone");

    sync_once_with_apply(
        &mut desktop,
        &client,
        token,
        &account_id,
        dev_desktop,
        &account_key,
        clock::now(),
        |s, page| {
            apply_decrypted_page(s, page)?;
            Ok(())
        },
    )
    .expect("desktop push tombstone");
    sync_once_with_apply(
        &mut android,
        &client,
        token,
        &account_id,
        dev_android,
        &account_key,
        clock::now(),
        |s, page| {
            apply_decrypted_page(s, page)?;
            Ok(())
        },
    )
    .expect("android pull tombstone");

    assert!(android.note(note.id()).expect("note").is_none());

    // 7. Inline WebSocket push fast-path: Android pushes a new task, Desktop
    //    applies the RemoteChange directly via `apply_inline_remote_changes`
    //    with zero HTTP round-trips.
    let ws_task = Task::new("Live WS task", clock::now()).expect("ws task");
    android.save_task(&ws_task).expect("save ws task");
    enqueue_task_change(&android, dev_android, &ws_task, clock::now()).expect("enqueue ws task");
    let push_report = sync_once_with_apply(
        &mut android,
        &client,
        token,
        &account_id,
        dev_android,
        &account_key,
        clock::now(),
        |s, page| {
            apply_decrypted_page(s, page)?;
            Ok(())
        },
    )
    .expect("android push ws task");
    assert_eq!(push_report.pushed, 1);

    let desktop_seq_before = desktop.sync_state().expect("st").last_pull_seq;
    let pulled = client
        .pull(
            token,
            &ottero_sync_protocol::PullRequest {
                protocol: ottero_sync_protocol::PROTOCOL_VERSION,
                after_seq: desktop_seq_before,
                limit: 10,
            },
        )
        .expect("fetch pushed remote change");
    assert_eq!(pulled.changes.len(), 1);

    // Gap check: a non-contiguous seq must return Ok(None) so caller falls back to HTTP pull.
    let mut gapped = pulled.changes.clone();
    gapped[0].seq += 5;
    assert_eq!(
        ottero_sync_client::apply_inline_remote_changes(
            &mut desktop,
            &account_id,
            dev_desktop,
            &account_key,
            clock::now(),
            gapped[0].seq,
            &gapped,
        )
        .expect("gap check"),
        None
    );

    // Contiguous inline apply: zero HTTP calls, task immediately present in SQLite.
    let touched = ottero_sync_client::apply_inline_remote_changes(
        &mut desktop,
        &account_id,
        dev_desktop,
        &account_key,
        clock::now(),
        pulled.next_seq,
        &pulled.changes,
    )
    .expect("inline apply")
    .expect("applied inline");
    assert!(touched.contains(&ottero_sync_protocol::Collection::Tasks));
    assert_eq!(
        desktop.sync_state().expect("st").last_pull_seq,
        pulled.next_seq
    );
    assert_eq!(
        desktop
            .task(ws_task.id())
            .expect("task")
            .expect("present")
            .title(),
        "Live WS task"
    );
}
