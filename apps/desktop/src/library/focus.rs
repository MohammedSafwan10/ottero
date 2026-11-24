//! Focus session recording and queries on [`Library`].

use std::collections::HashMap;

use anyhow::Result;
use gpui_kit::Context;

use ottero_core::{FocusSession, FocusSessionId, TaskFocusStats, TaskId, Timestamp};

use super::{Library, LibraryEvent};

impl Library {
    // ---- Focus -----------------------------------------------------------

    pub fn record_focus_session(
        &mut self,
        session: &FocusSession,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.store.record_focus_session(session)?;
        self.record_sync(
            ottero_sync_protocol::Collection::Focus,
            &session.id.to_string(),
            &["record"],
            false,
            session,
        );
        cx.emit(LibraryEvent::FocusChanged);
        Ok(())
    }

    pub fn delete_focus_session(
        &mut self,
        id: FocusSessionId,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.store.delete_focus_session(id)?;
        self.record_sync(
            ottero_sync_protocol::Collection::Focus,
            &id.to_string(),
            &["record"],
            true,
            &serde_json::json!({}),
        );
        cx.emit(LibraryEvent::FocusChanged);
        Ok(())
    }

    pub fn focus_sessions_between(
        &self,
        start: Timestamp,
        end: Timestamp,
    ) -> Result<Vec<FocusSession>> {
        Ok(self.store.focus_sessions_between(start, end)?)
    }

    pub fn focus_stats_for_task(&self, task_id: TaskId) -> Result<TaskFocusStats> {
        Ok(self.store.focus_stats_for_task(task_id)?)
    }

    pub fn focus_totals_by_task(&self) -> Result<HashMap<TaskId, TaskFocusStats>> {
        Ok(self.store.focus_totals_by_task()?)
    }
}
