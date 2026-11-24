//! Domain record serialization, outbox enqueuing, and per-field LWW remote apply.

use ottero_core::event::{exceptions_from_code, exceptions_to_code};
use ottero_core::{
    Date, DateTime, Event, EventId, EventRecord, FocusSession, FocusSessionId, Habit, HabitId,
    HabitRecord, Note, NoteId, Occasion, Priority, Project, ProjectId, Recurrence, Schedule, Task,
    TaskId, TaskRecord, TaskStatus, Timestamp, VaultItemId,
};
use ottero_storage::{AccountWrap, OutboxEntry, SealedItem, Store, VaultMeta};
use ottero_sync_protocol::{ChangeId, Collection, Hlc, HlcClock, decode_blob, encode_blob};
use serde::{Deserialize, Serialize};

use crate::sync::client::SyncClientError;
use crate::sync::engine::DecryptedChange;

/// Reserved `Collection::Vault` item ID carrying the account-wrapped vault key
/// header (`vault_meta`) so synced Passwords decrypt seamlessly on other devices.
pub const VAULT_META_SYNC_ID: &str = "vault-meta";

/// Wire payload for [`VAULT_META_SYNC_ID`] inside `Collection::Vault` (v1).
/// Carries only the already-encrypted `account_wrapped` envelope (never a
/// plaintext key or local master-password columns).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncVaultMetaPayload {
    pub version: u32,
    pub account_email: String,
    pub account_wrapped_b64: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl SyncVaultMetaPayload {
    #[must_use]
    pub fn from_meta(meta: &VaultMeta) -> Option<Self> {
        let account = meta.account.as_ref()?;
        Some(Self {
            version: meta.version,
            account_email: account.email.clone(),
            account_wrapped_b64: encode_blob(&account.wrapped_key),
            created_at: meta.created_at,
            updated_at: meta.updated_at,
        })
    }
}

/// Wire payload for `Collection::Notes` (v1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncNotePayload {
    pub id: NoteId,
    pub title: String,
    pub body: String,
    pub pinned: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub deleted_at: Option<Timestamp>,
}

impl From<&Note> for SyncNotePayload {
    fn from(n: &Note) -> Self {
        Self {
            id: n.id(),
            title: n.title().to_owned(),
            body: n.body().to_owned(),
            pinned: n.is_pinned(),
            created_at: n.created_at(),
            updated_at: n.updated_at(),
            deleted_at: n.deleted_at(),
        }
    }
}

/// Wire payload for `Collection::Tasks` (v1), merged per-field by HLC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncTaskPayload {
    pub id: TaskId,
    pub title: String,
    pub notes: String,
    pub status: TaskStatus,
    pub priority: Priority,
    pub due: Option<Date>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub completed_at: Option<Timestamp>,
    pub deleted_at: Option<Timestamp>,
    #[serde(default)]
    pub recurrence: Option<Recurrence>,
    pub parent: Option<TaskId>,
    pub project: Option<ProjectId>,
    /// Fields modified in this change; empty means all fields (full snapshot).
    #[serde(default)]
    pub changed_fields: Vec<String>,
}

impl SyncTaskPayload {
    #[must_use]
    pub fn from_task(t: &Task, changed_fields: &[&str]) -> Self {
        let r = t.to_record();
        Self {
            id: r.id,
            title: r.title,
            notes: r.notes,
            status: r.status,
            priority: r.priority,
            due: r.due,
            created_at: r.created_at,
            updated_at: r.updated_at,
            completed_at: r.completed_at,
            deleted_at: r.deleted_at,
            recurrence: r.recurrence,
            parent: r.parent,
            project: r.project,
            changed_fields: changed_fields.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn includes(&self, field: &str) -> bool {
        self.changed_fields.is_empty() || self.changed_fields.iter().any(|f| f == field)
    }
}

impl From<&Task> for SyncTaskPayload {
    fn from(t: &Task) -> Self {
        Self::from_task(t, &[])
    }
}

/// Wire payload for `Collection::Projects` (v1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncProjectPayload {
    pub id: ProjectId,
    pub name: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl From<&Project> for SyncProjectPayload {
    fn from(p: &Project) -> Self {
        Self {
            id: p.id(),
            name: p.name().to_owned(),
            created_at: p.created_at(),
            updated_at: p.updated_at(),
        }
    }
}

/// Wire payload for `Collection::Events` (v1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncEventPayload {
    pub id: EventId,
    pub title: String,
    pub notes: String,
    pub location: String,
    pub all_day: bool,
    pub start: DateTime,
    pub end: DateTime,
    pub recurrence: Option<String>,
    pub until: Option<Date>,
    pub occasion: Option<String>,
    pub exceptions: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub deleted_at: Option<Timestamp>,
}

impl From<&Event> for SyncEventPayload {
    fn from(e: &Event) -> Self {
        let r = e.to_record();
        Self {
            id: r.id,
            title: r.title,
            notes: r.notes,
            location: r.location,
            all_day: r.all_day,
            start: r.start,
            end: r.end,
            recurrence: r.recurrence.map(|x| x.to_code().to_owned()),
            until: r.until,
            occasion: r.occasion.map(Occasion::to_code),
            exceptions: exceptions_to_code(&r.exceptions).unwrap_or_default(),
            created_at: r.created_at,
            updated_at: r.updated_at,
            deleted_at: r.deleted_at,
        }
    }
}

/// Wire payload for `Collection::Habits` (v1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncHabitPayload {
    pub id: HabitId,
    pub name: String,
    pub schedule: String,
    pub target: u16,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub archived_at: Option<Timestamp>,
}

impl From<&Habit> for SyncHabitPayload {
    fn from(h: &Habit) -> Self {
        let r = h.to_record();
        Self {
            id: r.id,
            name: r.name,
            schedule: r.schedule.to_code(),
            target: r.target,
            created_at: r.created_at,
            updated_at: r.updated_at,
            archived_at: r.archived_at,
        }
    }
}

/// Wire payload for `Collection::Vault` (v1): carries the already-sealed
/// vault item ciphertext (`vault_items.blob`), never plaintext secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncVaultPayload {
    pub id: VaultItemId,
    pub blob_b64: String,
    pub updated_at: i64,
}

impl From<&SealedItem> for SyncVaultPayload {
    fn from(item: &SealedItem) -> Self {
        Self {
            id: item.id,
            blob_b64: encode_blob(&item.blob),
            updated_at: item.updated_at,
        }
    }
}

/// Enqueues a local change into `sync_outbox`, advances `sync_state.last_hlc`,
/// and records the winning HLC for `fields` in `sync_field_clocks`.
#[allow(clippy::too_many_arguments)]
pub fn enqueue_local_change(
    store: &Store,
    device_id: &str,
    collection: Collection,
    item_id: &str,
    fields: &[&str],
    deleted: bool,
    payload: &impl Serialize,
    now: Timestamp,
) -> Result<Hlc, SyncClientError> {
    let mut state = store.sync_state()?;
    let mut clock = match &state.last_hlc {
        Some(last) => HlcClock::from_last(device_id, last)?,
        None => HlcClock::new(device_id)?,
    };
    let hlc = clock.tick(now.as_millisecond())?;
    let fields_json =
        serde_json::to_string(payload).map_err(|err| SyncClientError::Decode(err.to_string()))?;

    for field in fields {
        let _ = store.set_field_clock_if_newer(collection, item_id, field, &hlc)?;
    }

    store.enqueue_sync_change(&OutboxEntry {
        change_id: ChangeId::new(),
        collection,
        item_id: item_id.to_owned(),
        hlc: hlc.clone(),
        schema_version: 1,
        deleted,
        fields_json,
        created_at: now,
    })?;

    state.last_hlc = Some(hlc.clone());
    store.save_sync_state(&state)?;
    Ok(hlc)
}

/// Applies a full pull page in foreign-key dependency order (`Projects` →
/// `Tasks` → other collections) so `PRAGMA foreign_keys = ON` never fails on
/// out-of-order delivery. Returns the list of collections whose local state
/// changed.
pub fn apply_decrypted_page(
    store: &mut Store,
    changes: &[DecryptedChange],
) -> Result<Vec<Collection>, SyncClientError> {
    let mut order: Vec<usize> = (0..changes.len()).collect();
    order.sort_by_key(|&i| {
        let c = &changes[i];
        let rank = match c.collection {
            Collection::Projects if !c.deleted => 0,
            Collection::Vault if c.item_id == VAULT_META_SYNC_ID && !c.deleted => 0,
            Collection::Tasks if !c.deleted => 1,
            Collection::Notes
            | Collection::Events
            | Collection::Habits
            | Collection::Focus
            | Collection::Vault
            | Collection::Settings => 2,
            Collection::Tasks => 3,
            Collection::Projects => 4,
        };
        (rank, c.seq)
    });
    let mut touched = Vec::new();
    for i in order {
        let ch = &changes[i];
        if apply_decrypted_change(store, ch)? {
            touched.push(ch.collection);
        }
    }
    Ok(touched)
}

/// Applies one decrypted remote change into `store` using HLC last-writer-wins
/// (per-field for Tasks, per-record for Notes/Projects/Focus). Returns `true`
/// if local state changed.
pub fn apply_decrypted_change(
    store: &mut Store,
    change: &DecryptedChange,
) -> Result<bool, SyncClientError> {
    match change.collection {
        Collection::Notes => apply_note(store, change),
        Collection::Tasks => apply_task(store, change),
        Collection::Projects => apply_project(store, change),
        Collection::Focus => apply_focus(store, change),
        Collection::Events => apply_event(store, change),
        Collection::Habits => apply_habit(store, change),
        Collection::Vault => apply_vault(store, change),
        Collection::Settings => Ok(false),
    }
}

fn apply_note(store: &mut Store, change: &DecryptedChange) -> Result<bool, SyncClientError> {
    let id: NoteId = change
        .item_id
        .parse()
        .map_err(|_| SyncClientError::Decode("invalid note id".into()))?;
    if !store.set_field_clock_if_newer(Collection::Notes, &change.item_id, "record", &change.hlc)? {
        return Ok(false);
    }
    if change.deleted {
        store.delete_note_permanently(id)?;
        return Ok(true);
    }
    let payload: SyncNotePayload = serde_json::from_str(&change.payload_json)
        .map_err(|err| SyncClientError::Decode(err.to_string()))?;
    if payload.id != id {
        return Err(SyncClientError::Decode("note payload id mismatch".into()));
    }
    let note = Note::from_parts(
        payload.id,
        payload.title,
        payload.body,
        payload.pinned,
        payload.created_at,
        payload.updated_at,
        payload.deleted_at,
    );
    store.save_note(&note)?;
    Ok(true)
}

fn apply_task(store: &mut Store, change: &DecryptedChange) -> Result<bool, SyncClientError> {
    let id: TaskId = change
        .item_id
        .parse()
        .map_err(|_| SyncClientError::Decode("invalid task id".into()))?;

    if change.deleted {
        if !store.set_field_clock_if_newer(
            Collection::Tasks,
            &change.item_id,
            "deleted",
            &change.hlc,
        )? {
            return Ok(false);
        }
        store.delete_task_permanently(id)?;
        return Ok(true);
    }

    // Never resurrect a task that was permanently deleted at a newer/equal HLC.
    if let Some(deleted_hlc) = store.field_clock(Collection::Tasks, &change.item_id, "deleted")?
        && deleted_hlc >= change.hlc
    {
        return Ok(false);
    }

    let remote: SyncTaskPayload = serde_json::from_str(&change.payload_json)
        .map_err(|err| SyncClientError::Decode(err.to_string()))?;
    if remote.id != id {
        return Err(SyncClientError::Decode("task payload id mismatch".into()));
    }

    let existing = store.task(id)?;
    let mut changed = existing.is_none();
    let mut rec = existing.map_or_else(
        || TaskRecord {
            id: remote.id,
            title: remote.title.clone(),
            notes: remote.notes.clone(),
            status: remote.status,
            priority: remote.priority,
            due: remote.due,
            created_at: remote.created_at,
            updated_at: remote.updated_at,
            completed_at: remote.completed_at,
            deleted_at: remote.deleted_at,
            recurrence: remote.recurrence,
            parent: remote.parent,
            project: remote.project,
        },
        |t| t.to_record(),
    );

    if remote.includes("title")
        && store.set_field_clock_if_newer(
            Collection::Tasks,
            &change.item_id,
            "title",
            &change.hlc,
        )?
    {
        rec.title = remote.title.clone();
        changed = true;
    }
    if remote.includes("notes")
        && store.set_field_clock_if_newer(
            Collection::Tasks,
            &change.item_id,
            "notes",
            &change.hlc,
        )?
    {
        rec.notes = remote.notes.clone();
        changed = true;
    }
    if remote.includes("status")
        && store.set_field_clock_if_newer(
            Collection::Tasks,
            &change.item_id,
            "status",
            &change.hlc,
        )?
    {
        rec.status = remote.status;
        rec.completed_at = remote.completed_at;
        changed = true;
    }
    if remote.includes("priority")
        && store.set_field_clock_if_newer(
            Collection::Tasks,
            &change.item_id,
            "priority",
            &change.hlc,
        )?
    {
        rec.priority = remote.priority;
        changed = true;
    }
    if remote.includes("due")
        && store.set_field_clock_if_newer(Collection::Tasks, &change.item_id, "due", &change.hlc)?
    {
        rec.due = remote.due;
        changed = true;
    }
    if remote.includes("deleted_at")
        && store.set_field_clock_if_newer(
            Collection::Tasks,
            &change.item_id,
            "deleted_at",
            &change.hlc,
        )?
    {
        rec.deleted_at = remote.deleted_at;
        changed = true;
    }
    if remote.includes("recurrence")
        && store.set_field_clock_if_newer(
            Collection::Tasks,
            &change.item_id,
            "recurrence",
            &change.hlc,
        )?
    {
        rec.recurrence = remote.recurrence;
        changed = true;
    }
    if remote.includes("project")
        && store.set_field_clock_if_newer(
            Collection::Tasks,
            &change.item_id,
            "project",
            &change.hlc,
        )?
    {
        rec.project = remote.project;
        changed = true;
    }

    if changed {
        // Guard optional foreign keys against out-of-order or deleted parents.
        if let Some(pid) = rec.project
            && !store.projects()?.iter().any(|(p, _)| p.id() == pid)
        {
            rec.project = None;
        }
        if let Some(parent_id) = rec.parent
            && store.task(parent_id)?.is_none()
        {
            rec.parent = None;
        }
        rec.updated_at = rec.updated_at.max(remote.updated_at);
        let merged = Task::from_record(rec);
        store.save_task(&merged)?;
    }
    Ok(changed)
}

fn apply_project(store: &mut Store, change: &DecryptedChange) -> Result<bool, SyncClientError> {
    let id: ProjectId = change
        .item_id
        .parse()
        .map_err(|_| SyncClientError::Decode("invalid project id".into()))?;
    if !store.set_field_clock_if_newer(
        Collection::Projects,
        &change.item_id,
        "record",
        &change.hlc,
    )? {
        return Ok(false);
    }
    if change.deleted {
        store.delete_project(id)?;
        return Ok(true);
    }
    let payload: SyncProjectPayload = serde_json::from_str(&change.payload_json)
        .map_err(|err| SyncClientError::Decode(err.to_string()))?;
    if payload.id != id {
        return Err(SyncClientError::Decode(
            "project payload id mismatch".into(),
        ));
    }
    let project = Project::from_parts(
        payload.id,
        payload.name,
        payload.created_at,
        payload.updated_at,
    );
    store.save_project(&project)?;
    Ok(true)
}

fn apply_focus(store: &mut Store, change: &DecryptedChange) -> Result<bool, SyncClientError> {
    let id: FocusSessionId = change
        .item_id
        .parse()
        .map_err(|_| SyncClientError::Decode("invalid focus session id".into()))?;
    if !store.set_field_clock_if_newer(Collection::Focus, &change.item_id, "record", &change.hlc)? {
        return Ok(false);
    }
    if change.deleted {
        store.delete_focus_session(id)?;
        return Ok(true);
    }
    let mut session: FocusSession = serde_json::from_str(&change.payload_json)
        .map_err(|err| SyncClientError::Decode(err.to_string()))?;
    if session.id != id {
        return Err(SyncClientError::Decode("focus payload id mismatch".into()));
    }
    // Guard optional task FK against out-of-order or deleted tasks.
    if let Some(tid) = session.task_id
        && store.task(tid)?.is_none()
    {
        session.task_id = None;
    }
    store.record_focus_session(&session)?;
    Ok(true)
}

fn apply_event(store: &mut Store, change: &DecryptedChange) -> Result<bool, SyncClientError> {
    let id: EventId = change
        .item_id
        .parse()
        .map_err(|_| SyncClientError::Decode("invalid event id".into()))?;
    if !store.set_field_clock_if_newer(
        Collection::Events,
        &change.item_id,
        "record",
        &change.hlc,
    )? {
        return Ok(false);
    }
    if change.deleted {
        store.delete_event_permanently(id)?;
        return Ok(true);
    }
    let p: SyncEventPayload = serde_json::from_str(&change.payload_json)
        .map_err(|err| SyncClientError::Decode(err.to_string()))?;
    if p.id != id {
        return Err(SyncClientError::Decode("event payload id mismatch".into()));
    }
    let event = Event::from_record(EventRecord {
        id: p.id,
        title: p.title,
        notes: p.notes,
        location: p.location,
        all_day: p.all_day,
        start: p.start,
        end: p.end,
        recurrence: p.recurrence.as_deref().and_then(Recurrence::from_code),
        until: p.until,
        occasion: p.occasion.as_deref().and_then(Occasion::from_code),
        exceptions: exceptions_from_code(&p.exceptions),
        created_at: p.created_at,
        updated_at: p.updated_at,
        deleted_at: p.deleted_at,
    });
    store.save_event(&event)?;
    Ok(true)
}

fn apply_habit(store: &mut Store, change: &DecryptedChange) -> Result<bool, SyncClientError> {
    let id: HabitId = change
        .item_id
        .parse()
        .map_err(|_| SyncClientError::Decode("invalid habit id".into()))?;
    if !store.set_field_clock_if_newer(
        Collection::Habits,
        &change.item_id,
        "record",
        &change.hlc,
    )? {
        return Ok(false);
    }
    if change.deleted {
        store.delete_habit(id)?;
        return Ok(true);
    }
    let p: SyncHabitPayload = serde_json::from_str(&change.payload_json)
        .map_err(|err| SyncClientError::Decode(err.to_string()))?;
    if p.id != id {
        return Err(SyncClientError::Decode("habit payload id mismatch".into()));
    }
    let schedule = Schedule::from_code(&p.schedule)
        .ok_or_else(|| SyncClientError::Decode("invalid habit schedule".into()))?;
    let habit = Habit::from_record(HabitRecord {
        id: p.id,
        name: p.name,
        schedule,
        target: p.target,
        created_at: p.created_at,
        updated_at: p.updated_at,
        archived_at: p.archived_at,
    });
    store.save_habit(&habit)?;
    Ok(true)
}

fn apply_vault(store: &mut Store, change: &DecryptedChange) -> Result<bool, SyncClientError> {
    if change.item_id == VAULT_META_SYNC_ID {
        return apply_vault_meta(store, change);
    }
    let id: VaultItemId = change
        .item_id
        .parse()
        .map_err(|_| SyncClientError::Decode("invalid vault item id".into()))?;
    if !store.set_field_clock_if_newer(Collection::Vault, &change.item_id, "record", &change.hlc)? {
        return Ok(false);
    }
    if change.deleted {
        store.delete_vault_items(&[id])?;
        return Ok(true);
    }
    let p: SyncVaultPayload = serde_json::from_str(&change.payload_json)
        .map_err(|err| SyncClientError::Decode(err.to_string()))?;
    if p.id != id {
        return Err(SyncClientError::Decode("vault payload id mismatch".into()));
    }
    let blob = decode_blob(&p.blob_b64)?;
    store.save_vault_items(&[SealedItem {
        id: p.id,
        blob,
        updated_at: p.updated_at,
    }])?;
    Ok(true)
}

fn apply_vault_meta(store: &mut Store, change: &DecryptedChange) -> Result<bool, SyncClientError> {
    if change.deleted {
        return Ok(false);
    }
    let p: SyncVaultMetaPayload = serde_json::from_str(&change.payload_json)
        .map_err(|err| SyncClientError::Decode(err.to_string()))?;
    if p.version != 1 || p.account_email.trim().is_empty() {
        return Err(SyncClientError::Decode("invalid vault_meta payload".into()));
    }
    let wrapped_key = decode_blob(&p.account_wrapped_b64)?;
    if wrapped_key.is_empty() {
        return Err(SyncClientError::Decode(
            "empty vault_meta wrapped key".into(),
        ));
    }
    if let Some(existing) = store.vault_meta()? {
        let same_empty_account_vault = existing.master.is_none()
            && existing
                .account
                .as_ref()
                .is_some_and(|a| a.email.eq_ignore_ascii_case(&p.account_email))
            && store.vault_items()?.is_empty();
        if !same_empty_account_vault {
            return Ok(false);
        }
    }
    if !store.set_field_clock_if_newer(
        Collection::Vault,
        VAULT_META_SYNC_ID,
        "record",
        &change.hlc,
    )? {
        return Ok(false);
    }
    store.save_vault_meta(&VaultMeta {
        version: p.version,
        master: None,
        created_at: p.created_at,
        updated_at: p.updated_at,
        recovery_salt: None,
        recovery_wrapped: None,
        account: Some(AccountWrap {
            email: p.account_email.trim().to_ascii_lowercase(),
            wrapped_key,
        }),
    })?;
    Ok(true)
}

/// Enqueues every local record (for first sync or recovery after `410 CursorExpired`).
pub fn enqueue_full_snapshot(
    store: &Store,
    device_id: &str,
    now: Timestamp,
) -> Result<usize, SyncClientError> {
    let mut count = 0usize;
    for trashed in [false, true] {
        for summary in store.list_notes(trashed)? {
            if let Some(note) = store.note(summary.id)? {
                enqueue_local_change(
                    store,
                    device_id,
                    Collection::Notes,
                    &note.id().to_string(),
                    &["record"],
                    false,
                    &SyncNotePayload::from(&note),
                    now,
                )?;
                count += 1;
            }
        }
    }
    for (project, _) in store.projects()? {
        enqueue_local_change(
            store,
            device_id,
            Collection::Projects,
            &project.id().to_string(),
            &["record"],
            false,
            &SyncProjectPayload::from(&project),
            now,
        )?;
        count += 1;
    }
    for filter in [
        ottero_storage::TaskFilter::Open,
        ottero_storage::TaskFilter::Closed,
        ottero_storage::TaskFilter::Trash,
    ] {
        for task in store.list_tasks(filter)? {
            enqueue_local_change(
                store,
                device_id,
                Collection::Tasks,
                &task.id().to_string(),
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
                &SyncTaskPayload::from(&task),
                now,
            )?;
            count += 1;
        }
    }
    for archived in [false, true] {
        for habit in store.habits(archived)? {
            enqueue_local_change(
                store,
                device_id,
                Collection::Habits,
                &habit.id().to_string(),
                &["record"],
                false,
                &SyncHabitPayload::from(&habit),
                now,
            )?;
            count += 1;
        }
    }
    if let Some(meta) = store.vault_meta()?
        && let Some(payload) = SyncVaultMetaPayload::from_meta(&meta)
    {
        enqueue_local_change(
            store,
            device_id,
            Collection::Vault,
            VAULT_META_SYNC_ID,
            &["record"],
            false,
            &payload,
            now,
        )?;
        count += 1;
    }
    for item in store.vault_items()? {
        enqueue_local_change(
            store,
            device_id,
            Collection::Vault,
            &item.id.to_string(),
            &["record"],
            false,
            &SyncVaultPayload::from(&item),
            now,
        )?;
        count += 1;
    }
    Ok(count)
}
