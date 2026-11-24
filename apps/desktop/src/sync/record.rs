//! Domain record serialization, outbox enqueuing, and per-field LWW remote apply.

use ottero_core::{
    Date, FocusSession, FocusSessionId, Note, NoteId, Priority, Project, ProjectId, Task, TaskId,
    TaskRecord, TaskStatus, Timestamp,
};
use ottero_storage::{OutboxEntry, Store};
use ottero_sync_protocol::{ChangeId, Collection, Hlc, HlcClock};
use serde::{Deserialize, Serialize};

use crate::sync::client::SyncClientError;
use crate::sync::engine::DecryptedChange;

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
        Collection::Events | Collection::Habits | Collection::Settings | Collection::Vault => {
            Ok(false)
        }
    }
}

fn apply_note(store: &mut Store, change: &DecryptedChange) -> Result<bool, SyncClientError> {
    if !store.set_field_clock_if_newer(Collection::Notes, &change.item_id, "record", &change.hlc)? {
        return Ok(false);
    }
    if change.deleted {
        let id: NoteId = change
            .item_id
            .parse()
            .map_err(|_| SyncClientError::Decode("invalid note id".into()))?;
        store.delete_note_permanently(id)?;
        return Ok(true);
    }
    let payload: SyncNotePayload = serde_json::from_str(&change.payload_json)
        .map_err(|err| SyncClientError::Decode(err.to_string()))?;
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

    let remote: SyncTaskPayload = serde_json::from_str(&change.payload_json)
        .map_err(|err| SyncClientError::Decode(err.to_string()))?;

    let existing = store.task(id)?;
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
            recurrence: None,
            parent: remote.parent,
            project: remote.project,
        },
        |t| t.to_record(),
    );

    let mut changed = store.task(id)?.is_none();
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
        rec.updated_at = rec.updated_at.max(remote.updated_at);
        let merged = Task::from_record(rec);
        store.save_task(&merged)?;
    }
    Ok(changed)
}

fn apply_project(store: &mut Store, change: &DecryptedChange) -> Result<bool, SyncClientError> {
    if !store.set_field_clock_if_newer(
        Collection::Projects,
        &change.item_id,
        "record",
        &change.hlc,
    )? {
        return Ok(false);
    }
    if change.deleted {
        let id: ProjectId = change
            .item_id
            .parse()
            .map_err(|_| SyncClientError::Decode("invalid project id".into()))?;
        store.delete_project(id)?;
        return Ok(true);
    }
    let payload: SyncProjectPayload = serde_json::from_str(&change.payload_json)
        .map_err(|err| SyncClientError::Decode(err.to_string()))?;
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
    if !store.set_field_clock_if_newer(Collection::Focus, &change.item_id, "record", &change.hlc)? {
        return Ok(false);
    }
    if change.deleted {
        let id: FocusSessionId = change
            .item_id
            .parse()
            .map_err(|_| SyncClientError::Decode("invalid focus session id".into()))?;
        store.delete_focus_session(id)?;
        return Ok(true);
    }
    let session: FocusSession = serde_json::from_str(&change.payload_json)
        .map_err(|err| SyncClientError::Decode(err.to_string()))?;
    store.record_focus_session(&session)?;
    Ok(true)
}
