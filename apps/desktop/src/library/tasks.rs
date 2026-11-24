//! Tasks, subtasks, projects, and completion stats on [`Library`].

use std::collections::HashMap;

use anyhow::{Context as _, Result};
use gpui_kit::Context;

use ottero_core::{
    Date, Priority, Project, ProjectId, Recurrence, Task, TaskId, TaskStatus, clock,
    parse_quick_add,
};
use ottero_storage::{Progress, TaskCounts, TaskFilter};

use super::{Library, LibraryEvent};

impl Library {
    // ---- Tasks -----------------------------------------------------------

    pub fn tasks(&self, filter: TaskFilter) -> Result<Vec<Task>> {
        Ok(self.store.list_tasks(filter)?)
    }

    /// One task by ID, or `None` if it was deleted (or can't be read).
    pub fn tasks_by_id(&self, id: TaskId) -> Option<Task> {
        self.store.task(id).ok().flatten()
    }

    pub fn set_task_notes(
        &mut self,
        id: TaskId,
        notes: &str,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_task(id, cx, |t| Ok(t.set_notes(notes, clock::now())?))
    }

    pub fn task_counts(&self) -> Result<TaskCounts> {
        let today = clock::today();
        let tz = jiff::tz::TimeZone::system();
        let start = today.to_zoned(tz.clone())?.timestamp();
        let end = today.tomorrow()?.to_zoned(tz)?.timestamp();
        Ok(self.store.task_counts(today, start, end)?)
    }

    pub fn create_task(
        &mut self,
        title: &str,
        due: Option<Date>,
        cx: &mut Context<Self>,
    ) -> Result<TaskId> {
        let now = clock::now();
        let mut task = Task::new(title, now)?;
        task.set_due(due, now)?;
        self.store
            .save_task(&task)
            .context("Couldn't create the task")?;
        self.record_sync(
            ottero_sync_protocol::Collection::Tasks,
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
            &crate::sync::SyncTaskPayload::from(&task),
        );
        cx.emit(LibraryEvent::TasksChanged);
        Ok(task.id())
    }

    /// Checkbox: completes or reopens. Completing a repeating task also
    /// creates its next instance, in the same transaction.
    pub fn toggle_task(&mut self, id: TaskId, cx: &mut Context<Self>) -> Result<()> {
        let mut task = self.store.task(id)?.context("That task no longer exists")?;
        if task.status().is_open() {
            self.complete_task(task, cx)
        } else {
            if task.toggle_done(clock::now())? {
                self.store
                    .save_task(&task)
                    .context("Couldn't save the task")?;
                self.record_sync(
                    ottero_sync_protocol::Collection::Tasks,
                    &task.id().to_string(),
                    &["status"],
                    false,
                    &crate::sync::SyncTaskPayload::from_task(&task, &["status"]),
                );
                cx.emit(LibraryEvent::TasksChanged);
            }
            Ok(())
        }
    }

    fn complete_task(&mut self, mut task: Task, cx: &mut Context<Self>) -> Result<()> {
        let now = clock::now();
        if !task.complete(now)? {
            return Ok(());
        }
        let next = task.next_occurrence(clock::today(), now);
        task.clear_recurrence_quietly();
        match &next {
            Some(next) => self.store.save_tasks(&[&task, next]),
            None => self.store.save_task(&task),
        }
        .context("Couldn't save the task")?;
        self.record_sync(
            ottero_sync_protocol::Collection::Tasks,
            &task.id().to_string(),
            &["status", "recurrence"],
            false,
            &crate::sync::SyncTaskPayload::from_task(&task, &["status", "recurrence"]),
        );
        if let Some(next) = &next {
            self.record_sync(
                ottero_sync_protocol::Collection::Tasks,
                &next.id().to_string(),
                &[
                    "title",
                    "notes",
                    "status",
                    "priority",
                    "due",
                    "deleted_at",
                    "recurrence",
                    "project",
                ],
                false,
                &crate::sync::SyncTaskPayload::from(next),
            );
        }
        cx.emit(LibraryEvent::TasksChanged);
        Ok(())
    }

    /// Creates a task from quick-add text ("Pay rent tomorrow !high #Home").
    /// `default_due` applies when the text has no date; `project` is the
    /// current project view (a `#Project` in the text wins if it exists).
    pub fn quick_add_task(
        &mut self,
        input: &str,
        default_due: Option<Date>,
        project: Option<ProjectId>,
        cx: &mut Context<Self>,
    ) -> Result<TaskId> {
        let now = clock::now();
        let today = clock::today();
        let parsed = parse_quick_add(input, today);
        let mut title = parsed.title.clone();
        let mut project = project;
        if let Some(name) = &parsed.project {
            match self.store.find_project_by_name(name)? {
                Some(p) => project = Some(p.id()),
                // Unknown project: keep the word in the title rather than guess.
                None => title = format!("{title} #{}", name.replace(' ', "_")),
            }
        }
        let mut task = Task::new(&title, now)?;
        task.set_due(parsed.due.or(default_due), now)?;
        if let Some(p) = parsed.priority {
            task.set_priority(p, now)?;
        }
        task.set_recurrence(parsed.recurrence, today, now)?;
        task.set_project(project, now)?;
        self.store
            .save_task(&task)
            .context("Couldn't create the task")?;
        self.record_sync(
            ottero_sync_protocol::Collection::Tasks,
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
            &crate::sync::SyncTaskPayload::from(&task),
        );
        cx.emit(LibraryEvent::TasksChanged);
        Ok(task.id())
    }

    pub fn set_task_recurrence(
        &mut self,
        id: TaskId,
        rule: Option<Recurrence>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_task(id, cx, |t| {
            Ok(t.set_recurrence(rule, clock::today(), clock::now())?)
        })
    }

    pub fn set_task_project(
        &mut self,
        id: TaskId,
        project: Option<ProjectId>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_task(id, cx, |t| Ok(t.set_project(project, clock::now())?))
    }

    // ---- Subtasks ----

    pub fn subtasks(&self, parent: TaskId) -> Vec<Task> {
        self.store.subtasks(parent).unwrap_or_default()
    }

    pub fn subtask_progress(&self) -> HashMap<TaskId, Progress> {
        self.store.subtask_progress().unwrap_or_default()
    }

    pub fn add_subtask(
        &mut self,
        parent: TaskId,
        title: &str,
        cx: &mut Context<Self>,
    ) -> Result<TaskId> {
        let task = Task::new_subtask(parent, title, clock::now())?;
        self.store
            .save_task(&task)
            .context("Couldn't add the subtask")?;
        cx.emit(LibraryEvent::TasksChanged);
        Ok(task.id())
    }

    pub fn delete_subtask(&mut self, id: TaskId, cx: &mut Context<Self>) -> Result<()> {
        self.store.delete_task_permanently(id)?;
        cx.emit(LibraryEvent::TasksChanged);
        Ok(())
    }

    // ---- Projects ----

    pub fn projects(&self) -> Vec<(Project, usize)> {
        self.store.projects().unwrap_or_default()
    }

    pub fn project(&self, id: ProjectId) -> Option<Project> {
        self.store.project(id).ok().flatten()
    }

    pub fn create_project(&mut self, name: &str, cx: &mut Context<Self>) -> Result<ProjectId> {
        let project = Project::new(name, clock::now())?;
        self.store
            .save_project(&project)
            .context("Couldn't create the project")?;
        self.record_sync(
            ottero_sync_protocol::Collection::Projects,
            &project.id().to_string(),
            &["record"],
            false,
            &crate::sync::SyncProjectPayload::from(&project),
        );
        cx.emit(LibraryEvent::TasksChanged);
        Ok(project.id())
    }

    pub fn rename_project(
        &mut self,
        id: ProjectId,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let mut project = self
            .store
            .project(id)?
            .context("That project no longer exists")?;
        if project.rename(name, clock::now())? {
            self.store.save_project(&project)?;
            self.record_sync(
                ottero_sync_protocol::Collection::Projects,
                &project.id().to_string(),
                &["record"],
                false,
                &crate::sync::SyncProjectPayload::from(&project),
            );
            cx.emit(LibraryEvent::TasksChanged);
        }
        Ok(())
    }

    /// Deletes the project; its tasks stay (without a project).
    pub fn delete_project(&mut self, id: ProjectId, cx: &mut Context<Self>) -> Result<()> {
        self.store.delete_project(id)?;
        self.record_sync(
            ottero_sync_protocol::Collection::Projects,
            &id.to_string(),
            &["record"],
            true,
            &serde_json::json!({}),
        );
        cx.emit(LibraryEvent::DeveloperChanged);
        cx.emit(LibraryEvent::TasksChanged);
        Ok(())
    }

    // ---- Stats ----

    /// Tasks completed on each of the last `days` local days, oldest first.
    pub fn completions_by_day(&self, days: i64) -> Vec<(Date, usize)> {
        let today = clock::today();
        let tz = jiff::tz::TimeZone::system();
        let start_day = today
            .checked_sub(jiff::Span::new().days(days - 1))
            .unwrap_or(today);
        let (Ok(start), Ok(end)) = (
            start_day.to_zoned(tz.clone()),
            today.tomorrow().and_then(|d| d.to_zoned(tz.clone())),
        ) else {
            return Vec::new();
        };
        let stamps = self
            .store
            .completed_between(start.timestamp(), end.timestamp())
            .unwrap_or_default();
        (0..days)
            .filter_map(|i| start_day.checked_add(jiff::Span::new().days(i)).ok())
            .map(|day| {
                let count = stamps
                    .iter()
                    .filter(|ts| ts.to_zoned(tz.clone()).date() == day)
                    .count();
                (day, count)
            })
            .collect()
    }

    /// Any status change. Moving to Done goes through the same path as the
    /// checkbox, so repeating tasks always roll over.
    pub fn set_task_status(
        &mut self,
        id: TaskId,
        status: TaskStatus,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if status == TaskStatus::Done {
            let task = self.store.task(id)?.context("That task no longer exists")?;
            if task.status() != TaskStatus::Done {
                return self.complete_task(task, cx);
            }
            return Ok(());
        }
        self.update_task(id, cx, |t| Ok(t.transition(status, clock::now())?))
    }

    pub fn set_task_due(
        &mut self,
        id: TaskId,
        due: Option<Date>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_task(id, cx, |t| Ok(t.set_due(due, clock::now())?))
    }

    pub fn set_task_priority(
        &mut self,
        id: TaskId,
        priority: Priority,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_task(id, cx, |t| Ok(t.set_priority(priority, clock::now())?))
    }

    pub fn rename_task(&mut self, id: TaskId, title: &str, cx: &mut Context<Self>) -> Result<()> {
        self.update_task(id, cx, |t| Ok(t.rename(title, clock::now())?))
    }

    pub fn trash_task(&mut self, id: TaskId, cx: &mut Context<Self>) -> Result<()> {
        self.update_task(id, cx, |t| {
            t.trash(clock::now());
            Ok(true)
        })
    }

    pub fn restore_task(&mut self, id: TaskId, cx: &mut Context<Self>) -> Result<()> {
        self.update_task(id, cx, |t| {
            t.restore(clock::now());
            Ok(true)
        })
    }

    pub fn empty_task_trash(&mut self, cx: &mut Context<Self>) -> Result<usize> {
        let n = self.store.empty_task_trash()?;
        cx.emit(LibraryEvent::TasksChanged);
        Ok(n)
    }

    pub fn delete_task_permanently(&mut self, id: TaskId, cx: &mut Context<Self>) -> Result<()> {
        self.store.delete_task_permanently(id)?;
        self.record_sync(
            ottero_sync_protocol::Collection::Tasks,
            &id.to_string(),
            &["deleted"],
            true,
            &serde_json::json!({}),
        );
        cx.emit(LibraryEvent::TasksChanged);
        Ok(())
    }

    fn update_task(
        &mut self,
        id: TaskId,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Task) -> Result<bool>,
    ) -> Result<()> {
        let mut task = self.store.task(id)?.context("That task no longer exists")?;
        if change(&mut task)? {
            self.store
                .save_task(&task)
                .context("Couldn't save the task")?;
            self.record_sync(
                ottero_sync_protocol::Collection::Tasks,
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
                &crate::sync::SyncTaskPayload::from(&task),
            );
            cx.emit(LibraryEvent::TasksChanged);
        }
        Ok(())
    }
}
