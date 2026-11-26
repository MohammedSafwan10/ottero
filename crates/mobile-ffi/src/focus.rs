//! Focus session persistence and cloud sync exposed to mobile clients.

use ottero_core::{FocusKind, FocusSession, FocusSessionId, clock};
use ottero_storage::Store;
use ottero_sync_client::enqueue_local_change;
use ottero_sync_protocol::Collection;

use crate::dto::FocusSessionItem;
use crate::error::MobileError;
use crate::sync;

fn to_dto(s: &FocusSession) -> FocusSessionItem {
    FocusSessionItem {
        id: s.id.to_string(),
        label: s.display_label().to_owned(),
        kind: s.kind.code().to_owned(),
        actual_secs: s.actual_secs,
        completed: s.completed,
        ended_at_ms: s.ended_at.as_millisecond(),
    }
}

pub(crate) fn record_session(
    store: &mut Store,
    label: &str,
    kind_code: &str,
    planned_secs: u32,
    actual_secs: u32,
    completed: bool,
) -> Result<FocusSessionItem, MobileError> {
    let kind = FocusKind::from_code(kind_code).unwrap_or(FocusKind::Focus);
    let now = clock::now();
    let started_at = now
        .checked_sub(jiff::Span::new().seconds(i64::from(actual_secs)))
        .unwrap_or(now);
    let session = FocusSession {
        id: FocusSessionId::new(),
        task_id: None,
        label: label.trim().to_owned(),
        kind,
        planned_secs,
        actual_secs,
        started_at,
        ended_at: now,
        completed,
    };
    store.record_focus_session(&session)?;
    if let Some(device_id) = sync::active_device_id(store) {
        let _ = enqueue_local_change(
            store,
            &device_id,
            Collection::Focus,
            &session.id.to_string(),
            &["record"],
            false,
            &session,
            now,
        );
    }
    Ok(to_dto(&session))
}

pub(crate) fn list_today_sessions(store: &Store) -> Result<Vec<FocusSessionItem>, MobileError> {
    let today = clock::today();
    let tz = jiff::tz::TimeZone::system();
    let start = today
        .at(0, 0, 0, 0)
        .to_zoned(tz.clone())
        .map_or_else(|_| clock::now(), |z| z.timestamp());
    let end = today
        .tomorrow()
        .unwrap_or(today)
        .at(0, 0, 0, 0)
        .to_zoned(tz)
        .map_or_else(|_| clock::now(), |z| z.timestamp());
    let sessions = store.focus_sessions_between(start, end)?;
    Ok(sessions.iter().map(to_dto).collect())
}
