use std::collections::HashMap;

use ottero_core::{
    Date, Priority, Project, ProjectId, Recurrence, Task, TaskId, TaskRecord, TaskStatus, Timestamp,
};
use rusqlite::{OptionalExtension, Row, params};

use crate::codec::{
    date_from_sql, date_to_sql, opt_ts_from_sql, ts_from_sql, ts_to_sql, uuid_from_sql,
};
use crate::{StorageError, StorageResult, Store};

/// Which top-level tasks a list shows (subtasks never appear in lists).
/// Dates are passed in so results are deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskFilter {
    /// Open tasks due today or earlier (includes overdue).
    Today(Date),
    /// Open tasks due after the given day.
    Upcoming(Date),
    /// Every open task.
    Open,
    /// Open tasks in a project.
    Project(ProjectId),
    /// Done or cancelled, most recently finished first.
    Closed,
    /// Trashed tasks.
    Trash,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskCounts {
    pub open: usize,
    pub due_today: usize,
    pub overdue: usize,
    pub completed_today: usize,
}

/// (done, total) subtasks of one task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

const COLUMNS: &str = "id, title, notes, status, priority, due, created_at, updated_at, completed_at, deleted_at, recurrence, parent_id, project_id";

/// Open tasks: dated first (soonest first), then by priority, then oldest first.
const OPEN_ORDER: &str = "ORDER BY due IS NULL, due, priority DESC, created_at";
const OPEN: &str = "deleted_at IS NULL AND parent_id IS NULL AND status IN ('todo', 'in_progress')";

impl Store {
    pub fn save_task(&mut self, task: &Task) -> StorageResult<()> {
        let r = task.to_record();
        self.conn.execute(
            "INSERT INTO tasks (id, title, notes, status, priority, due, created_at, updated_at, completed_at, deleted_at, recurrence, parent_id, project_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT (id) DO UPDATE SET
                title = excluded.title,
                notes = excluded.notes,
                status = excluded.status,
                priority = excluded.priority,
                due = excluded.due,
                updated_at = excluded.updated_at,
                completed_at = excluded.completed_at,
                deleted_at = excluded.deleted_at,
                recurrence = excluded.recurrence,
                parent_id = excluded.parent_id,
                project_id = excluded.project_id",
            params![
                r.id.to_string(),
                r.title,
                r.notes,
                r.status.as_str(),
                r.priority.as_i64(),
                r.due.map(date_to_sql),
                ts_to_sql(r.created_at),
                ts_to_sql(r.updated_at),
                r.completed_at.map(ts_to_sql),
                r.deleted_at.map(ts_to_sql),
                r.recurrence.map(Recurrence::to_code),
                r.parent.map(|p| p.to_string()),
                r.project.map(|p| p.to_string()),
            ],
        )?;
        Ok(())
    }

    /// Saves several tasks in one transaction (e.g. complete + next occurrence).
    pub fn save_tasks(&mut self, tasks: &[&Task]) -> StorageResult<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = tasks.iter().try_for_each(|task| self.save_task(task));
        match result {
            Ok(()) => self.conn.execute_batch("COMMIT")?,
            Err(_) => self.conn.execute_batch("ROLLBACK")?,
        }
        result
    }

    pub fn task(&self, id: TaskId) -> StorageResult<Option<Task>> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM tasks WHERE id = ?1"),
                [id.to_string()],
                raw_task,
            )
            .optional()?
            .map(RawTask::into_task)
            .transpose()
    }

    pub fn list_tasks(&self, filter: TaskFilter) -> StorageResult<Vec<Task>> {
        let (sql, arg) = match filter {
            TaskFilter::Today(day) => (
                format!("SELECT {COLUMNS} FROM tasks WHERE {OPEN} AND due <= ?1 {OPEN_ORDER}"),
                Some(date_to_sql(day)),
            ),
            TaskFilter::Upcoming(day) => (
                format!("SELECT {COLUMNS} FROM tasks WHERE {OPEN} AND due > ?1 {OPEN_ORDER}"),
                Some(date_to_sql(day)),
            ),
            TaskFilter::Open => (format!("SELECT {COLUMNS} FROM tasks WHERE {OPEN} {OPEN_ORDER}"), None),
            TaskFilter::Project(id) => (
                format!("SELECT {COLUMNS} FROM tasks WHERE {OPEN} AND project_id = ?1 {OPEN_ORDER}"),
                Some(id.to_string()),
            ),
            TaskFilter::Closed => (
                format!(
                    "SELECT {COLUMNS} FROM tasks
                     WHERE deleted_at IS NULL AND parent_id IS NULL AND status IN ('done', 'cancelled')
                     ORDER BY coalesce(completed_at, updated_at) DESC"
                ),
                None,
            ),
            TaskFilter::Trash => (
                format!(
                    "SELECT {COLUMNS} FROM tasks WHERE deleted_at IS NOT NULL AND parent_id IS NULL ORDER BY deleted_at DESC"
                ),
                None,
            ),
        };
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = match arg {
            Some(arg) => stmt.query_map([arg], raw_task)?.collect::<Vec<_>>(),
            None => stmt.query_map([], raw_task)?.collect::<Vec<_>>(),
        };
        rows.into_iter()
            .map(|row| row.map_err(StorageError::from)?.into_task())
            .collect()
    }

    /// Subtasks of `parent`, in the order they were added.
    pub fn subtasks(&self, parent: TaskId) -> StorageResult<Vec<Task>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {COLUMNS} FROM tasks WHERE parent_id = ?1 AND deleted_at IS NULL ORDER BY created_at, id"
        ))?;
        let rows = stmt
            .query_map([parent.to_string()], raw_task)?
            .collect::<Vec<_>>();
        rows.into_iter()
            .map(|row| row.map_err(StorageError::from)?.into_task())
            .collect()
    }

    /// Subtask progress for every task that has subtasks.
    pub fn subtask_progress(&self) -> StorageResult<HashMap<TaskId, Progress>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT parent_id, count(*) FILTER (WHERE status = 'done'), count(*)
             FROM tasks WHERE parent_id IS NOT NULL AND deleted_at IS NULL GROUP BY parent_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let n = |v: i64| usize::try_from(v).unwrap_or(0);
        rows.map(|row| {
            let (id, done, total) = row?;
            Ok((
                TaskId::from_uuid(uuid_from_sql(&id, "tasks.parent_id")?),
                Progress {
                    done: n(done),
                    total: n(total),
                },
            ))
        })
        .collect()
    }

    /// Dashboard counters for top-level tasks. `day_start`/`day_end` bound
    /// "today" in local time, as UTC timestamps.
    pub fn task_counts(
        &self,
        today: Date,
        day_start: Timestamp,
        day_end: Timestamp,
    ) -> StorageResult<TaskCounts> {
        let today = date_to_sql(today);
        let (open, due_today, overdue, completed_today): (i64, i64, i64, i64) =
            self.conn.query_row(
                "SELECT
                count(*) FILTER (WHERE status IN ('todo', 'in_progress')),
                count(*) FILTER (WHERE status IN ('todo', 'in_progress') AND due = ?1),
                count(*) FILTER (WHERE status IN ('todo', 'in_progress') AND due < ?1),
                count(*) FILTER (WHERE status = 'done' AND completed_at >= ?2 AND completed_at < ?3)
             FROM tasks WHERE deleted_at IS NULL AND parent_id IS NULL",
                params![today, ts_to_sql(day_start), ts_to_sql(day_end)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        let n = |v: i64| usize::try_from(v).unwrap_or(0);
        Ok(TaskCounts {
            open: n(open),
            due_today: n(due_today),
            overdue: n(overdue),
            completed_today: n(completed_today),
        })
    }

    /// Completion times of top-level tasks finished in `[from, to)`, for charts.
    pub fn completed_between(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> StorageResult<Vec<Timestamp>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT completed_at FROM tasks
             WHERE deleted_at IS NULL AND parent_id IS NULL AND status = 'done'
               AND completed_at >= ?1 AND completed_at < ?2",
        )?;
        let rows = stmt.query_map(params![ts_to_sql(from), ts_to_sql(to)], |r| {
            r.get::<_, i64>(0)
        })?;
        rows.map(|ms| ts_from_sql(ms?, "tasks.completed_at"))
            .collect()
    }

    pub fn delete_task_permanently(&mut self, id: TaskId) -> StorageResult<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM tasks WHERE id = ?1", [id.to_string()])?
            > 0)
    }

    pub fn empty_task_trash(&mut self) -> StorageResult<usize> {
        Ok(self.conn.execute(
            "DELETE FROM tasks WHERE deleted_at IS NOT NULL AND parent_id IS NULL",
            [],
        )?)
    }

    // ---- Projects --------------------------------------------------------

    pub fn save_project(&mut self, project: &Project) -> StorageResult<()> {
        self.conn.execute(
            "INSERT INTO projects (id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (id) DO UPDATE SET name = excluded.name, updated_at = excluded.updated_at",
            params![
                project.id().to_string(),
                project.name(),
                ts_to_sql(project.created_at()),
                ts_to_sql(project.updated_at())
            ],
        )?;
        Ok(())
    }

    /// Projects by name, with their open top-level task counts.
    pub fn projects(&self) -> StorageResult<Vec<(Project, usize)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT p.id, p.name, p.created_at, p.updated_at,
                    (SELECT count(*) FROM tasks t WHERE t.project_id = p.id
                       AND t.deleted_at IS NULL AND t.parent_id IS NULL AND t.status IN ('todo', 'in_progress'))
             FROM projects p ORDER BY p.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
        rows.map(|row| {
            let (id, name, created, updated, open) = row?;
            let project = Project::from_parts(
                ProjectId::from_uuid(uuid_from_sql(&id, "projects.id")?),
                name,
                ts_from_sql(created, "projects.created_at")?,
                ts_from_sql(updated, "projects.updated_at")?,
            );
            Ok((project, usize::try_from(open).unwrap_or(0)))
        })
        .collect()
    }

    pub fn project(&self, id: ProjectId) -> StorageResult<Option<Project>> {
        Ok(self
            .projects()?
            .into_iter()
            .map(|(p, _)| p)
            .find(|p| p.id() == id))
    }

    /// Finds a project by name, case-insensitively (for `#Project` in quick add).
    pub fn find_project_by_name(&self, name: &str) -> StorageResult<Option<Project>> {
        let key = name.trim().to_lowercase();
        Ok(self
            .projects()?
            .into_iter()
            .map(|(p, _)| p)
            .find(|p| p.name().to_lowercase() == key))
    }

    /// Deletes a project. Its tasks stay, without a project.
    pub fn delete_project(&mut self, id: ProjectId) -> StorageResult<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM projects WHERE id = ?1", [id.to_string()])?
            > 0)
    }
}

struct RawTask {
    id: String,
    title: String,
    notes: String,
    status: String,
    priority: i64,
    due: Option<String>,
    created_at: i64,
    updated_at: i64,
    completed_at: Option<i64>,
    deleted_at: Option<i64>,
    recurrence: Option<String>,
    parent_id: Option<String>,
    project_id: Option<String>,
}

impl RawTask {
    fn into_task(self) -> StorageResult<Task> {
        let id_of = |s: Option<String>, col| s.map(|s| uuid_from_sql(&s, col)).transpose();
        Ok(Task::from_record(TaskRecord {
            id: TaskId::from_uuid(uuid_from_sql(&self.id, "tasks.id")?),
            title: self.title,
            notes: self.notes,
            status: TaskStatus::parse(&self.status).ok_or(StorageError::Corrupt("tasks.status"))?,
            priority: Priority::from_i64(self.priority)
                .ok_or(StorageError::Corrupt("tasks.priority"))?,
            due: date_from_sql(self.due)?,
            created_at: ts_from_sql(self.created_at, "tasks.created_at")?,
            updated_at: ts_from_sql(self.updated_at, "tasks.updated_at")?,
            completed_at: opt_ts_from_sql(self.completed_at, "tasks.completed_at")?,
            deleted_at: opt_ts_from_sql(self.deleted_at, "tasks.deleted_at")?,
            // An unknown rule (e.g. from a newer version) is dropped, not fatal.
            recurrence: self.recurrence.as_deref().and_then(Recurrence::from_code),
            parent: id_of(self.parent_id, "tasks.parent_id")?.map(TaskId::from_uuid),
            project: id_of(self.project_id, "tasks.project_id")?.map(ProjectId::from_uuid),
        }))
    }
}

fn raw_task(row: &Row<'_>) -> rusqlite::Result<RawTask> {
    Ok(RawTask {
        id: row.get(0)?,
        title: row.get(1)?,
        notes: row.get(2)?,
        status: row.get(3)?,
        priority: row.get(4)?,
        due: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        completed_at: row.get(8)?,
        deleted_at: row.get(9)?,
        recurrence: row.get(10)?,
        parent_id: row.get(11)?,
        project_id: row.get(12)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ottero_core::{Timestamp, clock};

    fn at(secs: i64) -> Timestamp {
        Timestamp::from_second(secs).expect("ts")
    }

    fn date(s: &str) -> Date {
        s.parse().expect("date")
    }

    fn titles(tasks: &[Task]) -> Vec<&str> {
        tasks.iter().map(Task::title).collect()
    }

    #[test]
    fn save_and_load_round_trip_including_updates() {
        let mut store = Store::open_in_memory().expect("store");
        let now = clock::now();
        let mut task = Task::new("Pay rent", now).expect("task");
        task.set_due(Some(date("2026-12-01")), now).expect("due");
        task.set_priority(Priority::High, now).expect("prio");
        store.save_task(&task).expect("save");
        assert_eq!(store.task(task.id()).expect("load").as_ref(), Some(&task));

        task.complete(now).expect("done");
        store.save_task(&task).expect("save");
        assert_eq!(store.task(task.id()).expect("load"), Some(task));
    }

    #[test]
    fn filters_split_tasks_by_due_date_and_status() {
        let mut store = Store::open_in_memory().expect("store");
        let today = date("2026-11-08");
        let mk = |title: &str, due: Option<&str>, prio: Priority, secs: i64| {
            let mut t = Task::new(title, at(secs)).expect("task");
            t.set_due(due.map(date), at(secs)).expect("due");
            t.set_priority(prio, at(secs)).expect("prio");
            t
        };
        let overdue = mk("Overdue", Some("2026-11-01"), Priority::None, 1);
        let today_low = mk("Today low", Some("2026-11-08"), Priority::Low, 2);
        let today_high = mk("Today high", Some("2026-11-08"), Priority::High, 3);
        let later = mk("Later", Some("2026-11-20"), Priority::None, 4);
        let undated = mk("Someday", None, Priority::None, 5);
        let mut done = mk("Done", Some("2026-11-08"), Priority::None, 6);
        done.complete(at(7)).expect("done");
        let mut trashed = mk("Trashed", Some("2026-11-08"), Priority::None, 8);
        trashed.trash(at(9));
        for t in [
            &overdue,
            &today_low,
            &today_high,
            &later,
            &undated,
            &done,
            &trashed,
        ] {
            store.save_task(t).expect("save");
        }

        assert_eq!(
            titles(&store.list_tasks(TaskFilter::Today(today)).expect("today")),
            ["Overdue", "Today high", "Today low"]
        );
        assert_eq!(
            titles(
                &store
                    .list_tasks(TaskFilter::Upcoming(today))
                    .expect("upcoming")
            ),
            ["Later"]
        );
        assert_eq!(
            titles(&store.list_tasks(TaskFilter::Open).expect("open")),
            ["Overdue", "Today high", "Today low", "Later", "Someday"]
        );
        assert_eq!(
            titles(&store.list_tasks(TaskFilter::Closed).expect("closed")),
            ["Done"]
        );
        assert_eq!(
            titles(&store.list_tasks(TaskFilter::Trash).expect("trash")),
            ["Trashed"]
        );

        let counts = store.task_counts(today, at(0), at(100)).expect("counts");
        assert_eq!(
            counts,
            TaskCounts {
                open: 5,
                due_today: 2,
                overdue: 1,
                completed_today: 1
            }
        );
    }

    #[test]
    fn repeat_project_and_subtasks_round_trip_and_stay_out_of_lists() {
        let mut store = Store::open_in_memory().expect("store");
        let today = date("2026-11-08");
        let home = Project::new("Home", at(1)).expect("project");
        store.save_project(&home).expect("save project");

        let mut parent = Task::new("Move house", at(1)).expect("task");
        parent.set_project(Some(home.id()), at(1)).expect("project");
        parent
            .set_recurrence(Some(Recurrence::WEEKLY), today, at(1))
            .expect("repeat");
        let mut sub_a = Task::new_subtask(parent.id(), "Book van", at(2)).expect("sub");
        let sub_b = Task::new_subtask(parent.id(), "Pack", at(3)).expect("sub");
        sub_a.complete(at(4)).expect("done");
        store.save_tasks(&[&parent, &sub_a, &sub_b]).expect("save");

        assert_eq!(store.task(parent.id()).expect("load"), Some(parent.clone()));
        assert_eq!(
            titles(&store.list_tasks(TaskFilter::Open).expect("open")),
            ["Move house"],
            "subtasks hidden"
        );
        assert_eq!(
            titles(
                &store
                    .list_tasks(TaskFilter::Project(home.id()))
                    .expect("project")
            ),
            ["Move house"]
        );
        assert_eq!(
            titles(&store.subtasks(parent.id()).expect("subs")),
            ["Book van", "Pack"]
        );
        assert_eq!(
            store.subtask_progress().expect("progress")[&parent.id()],
            Progress { done: 1, total: 2 }
        );

        let projects = store.projects().expect("projects");
        assert_eq!((projects[0].0.name(), projects[0].1), ("Home", 1));
        assert_eq!(
            store
                .find_project_by_name("home")
                .expect("find")
                .map(|p| p.id()),
            Some(home.id())
        );

        // Deleting the project keeps its task; deleting the parent removes subtasks.
        store.delete_project(home.id()).expect("delete");
        assert_eq!(
            store
                .task(parent.id())
                .expect("load")
                .expect("task")
                .project(),
            None
        );
        store.delete_task_permanently(parent.id()).expect("delete");
        assert!(store.subtasks(parent.id()).expect("subs").is_empty());
    }

    #[test]
    fn completed_between_feeds_charts() {
        let mut store = Store::open_in_memory().expect("store");
        for (title, secs) in [("a", 10), ("b", 20), ("c", 99)] {
            let mut t = Task::new(title, at(1)).expect("task");
            t.complete(at(secs)).expect("done");
            store.save_task(&t).expect("save");
        }
        assert_eq!(
            store.completed_between(at(0), at(50)).expect("range").len(),
            2
        );
    }

    #[test]
    fn database_rejects_invalid_status_values() {
        let store = Store::open_in_memory().expect("store");
        let result = store.conn.execute(
            "INSERT INTO tasks (id, title, status, created_at, updated_at) VALUES ('x', 'y', 'bogus', 0, 0)",
            [],
        );
        assert!(
            result.is_err(),
            "CHECK constraint must reject unknown status"
        );
    }

    #[test]
    fn corrupt_rows_surface_as_errors_not_panics() {
        let store = Store::open_in_memory().expect("store");
        store
            .conn
            .execute(
                "INSERT INTO tasks (id, title, status, due, created_at, updated_at)
                 VALUES (?1, 'y', 'todo', 'not-a-date', 0, 0)",
                [TaskId::new().to_string()],
            )
            .expect("insert");
        assert!(matches!(
            store.list_tasks(TaskFilter::Open),
            Err(StorageError::Corrupt("tasks.due"))
        ));
    }

    #[test]
    fn empty_trash_and_permanent_delete() {
        let mut store = Store::open_in_memory().expect("store");
        let keep = Task::new("Keep", at(1)).expect("task");
        let mut gone = Task::new("Gone", at(1)).expect("task");
        gone.trash(at(2));
        store.save_task(&keep).expect("save");
        store.save_task(&gone).expect("save");
        assert_eq!(store.empty_task_trash().expect("empty"), 1);
        assert!(store.delete_task_permanently(keep.id()).expect("delete"));
        assert!(
            !store
                .delete_task_permanently(keep.id())
                .expect("delete again")
        );
    }

    #[test]
    fn file_database_persists_across_reopen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested/ottero.sqlite3");
        let task = Task::new("Persist me", clock::now()).expect("task");
        {
            let mut store = Store::open(&path).expect("open");
            store.save_task(&task).expect("save");
        }
        let store = Store::open(&path).expect("reopen");
        assert_eq!(store.task(task.id()).expect("load"), Some(task));
        assert_eq!(store.schema_version().expect("version"), 13);
    }
}
