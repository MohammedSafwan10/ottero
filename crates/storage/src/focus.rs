//! Focus session persistence.

use std::collections::HashMap;

use ottero_core::{FocusKind, FocusSession, FocusSessionId, TaskFocusStats, TaskId, Timestamp};
use rusqlite::{Row, params};

use crate::codec::{ts_from_sql, ts_to_sql, uuid_from_sql};
use crate::{StorageError, StorageResult, Store};

impl Store {
    /// Persists a completed or stopped focus/break session.
    pub fn record_focus_session(&self, session: &FocusSession) -> StorageResult<()> {
        self.conn.execute(
            "INSERT INTO focus_sessions (
                id, task_id, label, kind, planned_secs, actual_secs, started_at, ended_at, completed
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                session.id.to_string(),
                session.task_id.map(|id| id.to_string()),
                session.label,
                session.kind.code(),
                i64::from(session.planned_secs),
                i64::from(session.actual_secs),
                ts_to_sql(session.started_at),
                ts_to_sql(session.ended_at),
                i64::from(session.completed),
            ],
        )?;
        Ok(())
    }

    /// Removes a focus session by ID.
    pub fn delete_focus_session(&self, id: FocusSessionId) -> StorageResult<()> {
        self.conn.execute(
            "DELETE FROM focus_sessions WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// Lists every recorded focus session (for full sync snapshots).
    pub fn all_focus_sessions(&self) -> StorageResult<Vec<FocusSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, label, kind, planned_secs, actual_secs, started_at, ended_at, completed
             FROM focus_sessions
             ORDER BY ended_at DESC",
        )?;
        let rows = stmt
            .query_map([], decode_session)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Lists focus sessions whose `ended_at` falls in `[start, end)`, newest first.
    pub fn focus_sessions_between(
        &self,
        start: Timestamp,
        end: Timestamp,
    ) -> StorageResult<Vec<FocusSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, label, kind, planned_secs, actual_secs, started_at, ended_at, completed
             FROM focus_sessions
             WHERE ended_at >= ?1 AND ended_at < ?2
             ORDER BY ended_at DESC",
        )?;
        let rows = stmt
            .query_map(params![ts_to_sql(start), ts_to_sql(end)], decode_session)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Aggregated focus time (Focus blocks only, not breaks) per task.
    pub fn focus_totals_by_task(&self) -> StorageResult<HashMap<TaskId, TaskFocusStats>> {
        let mut stmt = self.conn.prepare(
            "SELECT task_id, COALESCE(SUM(actual_secs), 0), COUNT(*)
             FROM focus_sessions
             WHERE task_id IS NOT NULL AND kind = 'focus'
             GROUP BY task_id",
        )?;
        let mut out = HashMap::new();
        let rows = stmt.query_map([], |row| {
            let task_id: String = row.get(0)?;
            let total_secs: i64 = row.get(1)?;
            let sessions: i64 = row.get(2)?;
            Ok((task_id, total_secs, sessions))
        })?;
        for row in rows {
            let (task_id_str, total_secs, sessions) = row?;
            let task_id = TaskId::from_uuid(uuid_from_sql(&task_id_str, "focus_sessions.task_id")?);
            out.insert(
                task_id,
                TaskFocusStats {
                    total_secs: u64::try_from(total_secs.max(0)).unwrap_or(0),
                    sessions: u32::try_from(sessions.max(0)).unwrap_or(u32::MAX),
                },
            );
        }
        Ok(out)
    }

    /// Aggregated focus time for a single task.
    pub fn focus_stats_for_task(&self, task_id: TaskId) -> StorageResult<TaskFocusStats> {
        let (total_secs, sessions): (i64, i64) = self.conn.query_row(
            "SELECT COALESCE(SUM(actual_secs), 0), COUNT(*)
             FROM focus_sessions
             WHERE task_id = ?1 AND kind = 'focus'",
            params![task_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(TaskFocusStats {
            total_secs: u64::try_from(total_secs.max(0)).unwrap_or(0),
            sessions: u32::try_from(sessions.max(0)).unwrap_or(u32::MAX),
        })
    }
}

fn decode_session(row: &Row<'_>) -> rusqlite::Result<FocusSession> {
    let id_str: String = row.get(0)?;
    let task_id_str: Option<String> = row.get(1)?;
    let label: String = row.get(2)?;
    let kind_str: String = row.get(3)?;
    let planned_secs: i64 = row.get(4)?;
    let actual_secs: i64 = row.get(5)?;
    let started_at_ms: i64 = row.get(6)?;
    let ended_at_ms: i64 = row.get(7)?;
    let completed: i64 = row.get(8)?;

    let to_sql_err = |err: StorageError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
    };

    let id =
        FocusSessionId::from_uuid(uuid_from_sql(&id_str, "focus_sessions.id").map_err(to_sql_err)?);
    let task_id = task_id_str
        .map(|s| uuid_from_sql(&s, "focus_sessions.task_id").map(TaskId::from_uuid))
        .transpose()
        .map_err(to_sql_err)?;
    let kind = FocusKind::from_code(&kind_str)
        .ok_or_else(|| to_sql_err(StorageError::Corrupt("focus_sessions.kind")))?;
    let started_at = ts_from_sql(started_at_ms, "focus_sessions.started_at").map_err(to_sql_err)?;
    let ended_at = ts_from_sql(ended_at_ms, "focus_sessions.ended_at").map_err(to_sql_err)?;

    Ok(FocusSession {
        id,
        task_id,
        label,
        kind,
        planned_secs: u32::try_from(planned_secs.max(1)).unwrap_or(u32::MAX),
        actual_secs: u32::try_from(actual_secs.max(0)).unwrap_or(u32::MAX),
        started_at,
        ended_at,
        completed: completed != 0,
    })
}

#[cfg(test)]
mod tests {
    use ottero_core::{FocusKind, FocusSession, FocusSessionId, Task};

    use crate::Store;

    fn ts(secs: i64) -> ottero_core::Timestamp {
        ottero_core::Timestamp::from_second(secs).expect("ts")
    }

    #[test]
    fn record_query_and_task_totals_round_trip() {
        let mut store = Store::open_in_memory().expect("db");
        let task = Task::new("Ship focus", ts(100)).expect("task");
        store.save_task(&task).expect("save task");

        let s1 = FocusSession {
            id: FocusSessionId::new(),
            task_id: Some(task.id()),
            label: task.title().to_owned(),
            kind: FocusKind::Focus,
            planned_secs: 1500,
            actual_secs: 1500,
            started_at: ts(1000),
            ended_at: ts(2500),
            completed: true,
        };
        let brk = FocusSession {
            id: FocusSessionId::new(),
            task_id: Some(task.id()),
            label: String::new(),
            kind: FocusKind::ShortBreak,
            planned_secs: 300,
            actual_secs: 300,
            started_at: ts(2500),
            ended_at: ts(2800),
            completed: true,
        };
        store.record_focus_session(&s1).expect("s1");
        store.record_focus_session(&brk).expect("brk");

        let listed = store.focus_sessions_between(ts(0), ts(5000)).expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, brk.id);
        assert_eq!(listed[1].id, s1.id);

        // Task totals count only Focus blocks, not breaks.
        let stats = store.focus_stats_for_task(task.id()).expect("stats");
        assert_eq!(stats.total_secs, 1500);
        assert_eq!(stats.sessions, 1);

        let all = store.focus_totals_by_task().expect("all");
        assert_eq!(all.get(&task.id()).copied(), Some(stats));

        // Deleting a session works.
        store.delete_focus_session(brk.id).expect("delete");
        assert_eq!(
            store
                .focus_sessions_between(ts(0), ts(5000))
                .expect("list")
                .len(),
            1
        );
    }
}
