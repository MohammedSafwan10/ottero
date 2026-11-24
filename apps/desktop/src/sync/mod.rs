//! Ottero Cloud end-to-end encrypted sync client and engine.

pub mod client;
pub mod engine;
pub mod record;

pub use client::{DEFAULT_SYNC_URL, SyncClient, SyncClientError};
pub use engine::{
    DecryptedChange, SyncCycleReport, ensure_collection_keys, sync_collection_keys, sync_once,
};
pub use record::{
    SyncEventPayload, SyncHabitPayload, SyncNotePayload, SyncProjectPayload, SyncTaskPayload,
    SyncVaultPayload, apply_decrypted_change, enqueue_full_snapshot, enqueue_local_change,
};

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use ottero_core::Timestamp;
    use ottero_crypto::SecretKey;
    use ottero_storage::{OutboxEntry, Store};
    use ottero_sync_protocol::{
        ChangeId, Collection, CollectionKeyWire, Hlc, KeysResponse, PullResponse, PushAccepted,
        PushRequest, PushResponse, PutKeysRequest, QuotaUsage, RemoteChange, SyncErrorCode,
        SyncErrorResponse, decode_blob, encode_blob,
    };

    use super::*;

    #[derive(Default)]
    struct FakeState {
        seq: i64,
        keys: Vec<CollectionKeyWire>,
        changes: Vec<RemoteChange>,
        expire_cursor_below: i64,
        tamper_next_pull: bool,
    }

    struct FakeSyncServer {
        url: String,
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeSyncServer {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let addr = listener.local_addr().expect("addr");
            let state = Arc::new(Mutex::new(FakeState::default()));
            let srv_state = Arc::clone(&state);

            thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let _ = handle_conn(stream, &srv_state);
                }
            });

            Self {
                url: format!("http://{addr}"),
                state,
            }
        }
    }

    fn handle_conn(
        mut stream: std::net::TcpStream,
        state: &Arc<Mutex<FakeState>>,
    ) -> std::io::Result<()> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let mut header_end = None;
        let mut content_len = 0usize;

        loop {
            let n = stream.read(&mut tmp)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
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

        let Some(end) = header_end else {
            return Ok(());
        };
        let req_line = String::from_utf8_lossy(&buf[..end])
            .lines()
            .next()
            .unwrap_or("")
            .to_owned();
        let body = &buf[end..end + content_len];
        let mut parts = req_line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let target = parts.next().unwrap_or("");

        let (status, json) = if method == "GET" && target == "/v1/sync/keys" {
            let st = state.lock().expect("lock");
            let resp = KeysResponse {
                keys: st.keys.clone(),
            };
            ("200 OK", serde_json::to_vec(&resp).expect("keys resp"))
        } else if method == "PUT" && target == "/v1/sync/keys" {
            let req: PutKeysRequest = serde_json::from_slice(body).expect("put keys json");
            let mut st = state.lock().expect("lock");
            for k in req.keys {
                if !st
                    .keys
                    .iter()
                    .any(|existing| existing.collection == k.collection)
                {
                    st.keys.push(k);
                }
            }
            let resp = KeysResponse {
                keys: st.keys.clone(),
            };
            ("200 OK", serde_json::to_vec(&resp).expect("put keys resp"))
        } else if method == "POST" && target == "/v1/sync/push" {
            let req: PushRequest = serde_json::from_slice(body).expect("push json");
            let mut st = state.lock().expect("lock");
            let mut accepted = Vec::new();
            for ch in req.changes {
                if let Some(existing) = st.changes.iter().find(|c| c.change_id == ch.change_id) {
                    accepted.push(PushAccepted {
                        change_id: ch.change_id,
                        seq: existing.seq,
                        duplicate: true,
                    });
                    continue;
                }
                st.seq += 1;
                let seq = st.seq;
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
                    created_at_ms: 1_760_000_000_000,
                });
                accepted.push(PushAccepted {
                    change_id: ch.change_id,
                    seq,
                    duplicate: false,
                });
            }
            let resp = PushResponse {
                accepted,
                latest_seq: st.seq,
                quota: QuotaUsage {
                    used_bytes: 1024,
                    limit_bytes: 5 * 1024 * 1024 * 1024,
                },
            };
            ("200 OK", serde_json::to_vec(&resp).expect("resp"))
        } else if method == "GET" && target.starts_with("/v1/sync/pull") {
            let after_seq: i64 = target
                .split('?')
                .nth(1)
                .unwrap_or("")
                .split('&')
                .find_map(|p| p.strip_prefix("after_seq="))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let mut st = state.lock().expect("lock");
            if after_seq < st.expire_cursor_below {
                let err = SyncErrorResponse {
                    code: SyncErrorCode::CursorExpired,
                    message: "cursor too old".into(),
                };
                ("410 Gone", serde_json::to_vec(&err).expect("err"))
            } else {
                let mut changes: Vec<RemoteChange> = st
                    .changes
                    .iter()
                    .filter(|c| c.seq > after_seq)
                    .cloned()
                    .collect();
                if st.tamper_next_pull
                    && let Some(first) = changes.first_mut()
                {
                    let mut raw = decode_blob(&first.blob_b64).expect("decode");
                    let last = raw.len() - 1;
                    raw[last] ^= 0xFF;
                    first.blob_b64 = encode_blob(&raw);
                    st.tamper_next_pull = false;
                }
                let next_seq = changes.last().map_or(after_seq, |c| c.seq);
                let resp = PullResponse {
                    changes,
                    next_seq,
                    has_more: false,
                    oldest_available_seq: st.expire_cursor_below,
                    quota: QuotaUsage {
                        used_bytes: 1024,
                        limit_bytes: 5 * 1024 * 1024 * 1024,
                    },
                };
                ("200 OK", serde_json::to_vec(&resp).expect("resp"))
            }
        } else {
            ("404 Not Found", b"{}".to_vec())
        };

        let header = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            json.len()
        );
        stream.write_all(header.as_bytes())?;
        stream.write_all(&json)?;
        stream.flush()?;
        Ok(())
    }

    fn ts(ms: i64) -> Timestamp {
        Timestamp::from_millisecond(ms).expect("ts")
    }

    #[test]
    fn two_devices_sync_end_to_end_and_reject_tampering_and_expired_cursors() {
        let server = FakeSyncServer::start();
        let client = SyncClient::new(&server.url).expect("client");
        let account_key = SecretKey::generate().expect("key");
        let now = ts(1_760_000_000_000);

        let mut store_a = Store::open_in_memory().expect("db a");
        let mut store_b = Store::open_in_memory().expect("db b");

        // Device A enqueues a private change; collection keys are exchanged automatically via `/v1/sync/keys`.
        let secret_payload = r#"{"title":"Bank PIN is 4829"}"#;
        let entry = OutboxEntry {
            change_id: ChangeId::new(),
            collection: Collection::Notes,
            item_id: "note-1".into(),
            hlc: Hlc::new(1_760_000_000_000, 0, "dev-a").expect("hlc"),
            schema_version: 1,
            deleted: false,
            fields_json: secret_payload.into(),
            created_at: now,
        };
        store_a.enqueue_sync_change(&entry).expect("enqueue");

        let rep_a = sync_once(
            &mut store_a,
            &client,
            "tok-a",
            "acct-1",
            "dev-a",
            &account_key,
            now,
        )
        .expect("sync a");
        assert_eq!(rep_a.pushed, 1);
        assert_eq!(rep_a.pulled, 0, "own echo skipped");
        assert_eq!(store_a.pending_sync_count().expect("count"), 0);

        // Verify the server never saw plaintext.
        {
            let st = server.state.lock().expect("lock");
            assert_eq!(st.changes.len(), 1);
            assert!(!st.changes[0].blob_b64.contains("4829"));
            let raw = decode_blob(&st.changes[0].blob_b64).expect("decode");
            assert!(!String::from_utf8_lossy(&raw).contains("4829"));
        }

        // Tampered blob on server is rejected by Device B.
        server.state.lock().expect("lock").tamper_next_pull = true;
        let err = sync_once(
            &mut store_b,
            &client,
            "tok-b",
            "acct-1",
            "dev-b",
            &account_key,
            now,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            SyncClientError::Crypto(ottero_crypto::CryptoError::Decrypt)
        ));
        assert_eq!(store_b.sync_state().expect("state").last_pull_seq, 0);

        // Clean pull succeeds and decrypts the exact payload on Device B.
        let rep_b = sync_once(
            &mut store_b,
            &client,
            "tok-b",
            "acct-1",
            "dev-b",
            &account_key,
            now,
        )
        .expect("sync b");
        assert_eq!(rep_b.pulled, 1);
        assert_eq!(rep_b.decrypted[0].payload_json, secret_payload);
        assert_eq!(store_b.sync_state().expect("state").last_pull_seq, 1);

        // CursorExpired (HTTP 410) flags needs_full_resync in SQLite.
        server.state.lock().expect("lock").expire_cursor_below = 50;
        let expired = sync_once(
            &mut store_b,
            &client,
            "tok-b",
            "acct-1",
            "dev-b",
            &account_key,
            now,
        )
        .unwrap_err();
        assert!(matches!(expired, SyncClientError::CursorExpired));
        assert!(store_b.sync_state().expect("state").needs_full_resync);
    }

    #[test]
    fn two_devices_converge_notes_and_merge_concurrent_task_fields_by_lww() {
        use ottero_core::{Note, Priority, Task};

        let server = FakeSyncServer::start();
        let client = SyncClient::new(&server.url).expect("client");
        let account_key = SecretKey::generate().expect("key");
        let t0 = ts(1_760_000_000_000);
        let t1 = ts(1_760_000_001_000);
        let t2 = ts(1_760_000_002_000);

        let mut store_a = Store::open_in_memory().expect("db a");
        let mut store_b = Store::open_in_memory().expect("db b");

        let _ = ensure_collection_keys(&store_a, &account_key, t0).expect("keys a");
        for wrapped in store_a.sync_collection_keys().expect("wrapped") {
            store_b.save_sync_collection_key(&wrapped).expect("save b");
        }

        // Device A creates a Note and a Task.
        let note = Note::new("Launch plan", "Encrypted sync works.", t0).expect("note");
        let mut task_a = Task::new("Ship sync", t0).expect("task");
        store_a.save_note(&note).expect("save note");
        store_a.save_task(&task_a).expect("save task");

        enqueue_local_change(
            &store_a,
            "dev-a",
            Collection::Notes,
            &note.id().to_string(),
            &["record"],
            false,
            &SyncNotePayload::from(&note),
            t0,
        )
        .expect("enqueue note");
        enqueue_local_change(
            &store_a,
            "dev-a",
            Collection::Tasks,
            &task_a.id().to_string(),
            &[
                "title",
                "notes",
                "status",
                "priority",
                "due",
                "deleted_at",
                "project",
            ],
            false,
            &SyncTaskPayload::from(&task_a),
            t0,
        )
        .expect("enqueue task");

        // A pushes, B pulls & applies.
        sync_once(
            &mut store_a,
            &client,
            "tok",
            "acct-1",
            "dev-a",
            &account_key,
            t0,
        )
        .expect("push a");
        let rep_b = sync_once(
            &mut store_b,
            &client,
            "tok",
            "acct-1",
            "dev-b",
            &account_key,
            t0,
        )
        .expect("pull b");
        for ch in &rep_b.decrypted {
            apply_decrypted_change(&mut store_b, ch).expect("apply b");
        }
        assert_eq!(
            store_b
                .note(note.id())
                .expect("note b")
                .expect("exists")
                .title(),
            "Launch plan"
        );
        let mut task_b = store_b.task(task_a.id()).expect("task b").expect("exists");
        assert_eq!(task_b.title(), "Ship sync");

        // Concurrent offline edits on different fields:
        // A renames the task at t1; B raises priority at t2.
        task_a.rename("Ship E2EE sync", t1).expect("rename a");
        store_a.save_task(&task_a).expect("save a");
        enqueue_local_change(
            &store_a,
            "dev-a",
            Collection::Tasks,
            &task_a.id().to_string(),
            &["title"],
            false,
            &SyncTaskPayload::from_task(&task_a, &["title"]),
            t1,
        )
        .expect("enqueue title");

        task_b.set_priority(Priority::High, t2).expect("prio b");
        store_b.save_task(&task_b).expect("save b");
        enqueue_local_change(
            &store_b,
            "dev-b",
            Collection::Tasks,
            &task_b.id().to_string(),
            &["priority"],
            false,
            &SyncTaskPayload::from_task(&task_b, &["priority"]),
            t2,
        )
        .expect("enqueue prio");

        // Both push & pull, then apply.
        let r1 = sync_once(
            &mut store_a,
            &client,
            "tok",
            "acct-1",
            "dev-a",
            &account_key,
            t2,
        )
        .expect("sync a 2");
        for ch in &r1.decrypted {
            apply_decrypted_change(&mut store_a, ch).expect("apply a 2");
        }
        let r2 = sync_once(
            &mut store_b,
            &client,
            "tok",
            "acct-1",
            "dev-b",
            &account_key,
            t2,
        )
        .expect("sync b 2");
        for ch in &r2.decrypted {
            apply_decrypted_change(&mut store_b, ch).expect("apply b 2");
        }
        let r3 = sync_once(
            &mut store_a,
            &client,
            "tok",
            "acct-1",
            "dev-a",
            &account_key,
            t2,
        )
        .expect("sync a 3");
        for ch in &r3.decrypted {
            apply_decrypted_change(&mut store_a, ch).expect("apply a 3");
        }

        let final_a = store_a.task(task_a.id()).expect("final a").expect("exists");
        let final_b = store_b.task(task_a.id()).expect("final b").expect("exists");
        assert_eq!(final_a.title(), "Ship E2EE sync");
        assert_eq!(final_a.priority(), Priority::High);
        assert_eq!(final_b.title(), "Ship E2EE sync");
        assert_eq!(final_b.priority(), Priority::High);
    }
}
