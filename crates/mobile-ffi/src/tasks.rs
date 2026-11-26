//! Tasks operations exposed to mobile clients.

use std::collections::HashMap;

use jiff::civil::Date;
use ottero_core::{Priority, Project, ProjectId, Task, TaskId, TaskStatus, clock, parse_quick_add};
use ottero_storage::{Progress, Store, TaskFilter};
use ottero_sync_client::{SyncProjectPayload, enqueue_local_change, enqueue_task_change};
use ottero_sync_protocol::Collection;

use crate::dto::{MobileTaskFilter, ProjectItem, TaskItem};
use crate::error::MobileError;
use crate::sync;

fn enqueue_task(store: &Store, task: &Task) {
    if let Some(device_id) = sync::active_device_id(store) {
        let _ = enqueue_task_change(store, &device_id, task, clock::now());
    }
}

pub(crate) fn task_to_dto_rich(
    t: &Task,
    today: Date,
    projects: &HashMap<ProjectId, String>,
    progress: &HashMap<TaskId, Progress>,
) -> TaskItem {
    let priority = match t.priority() {
        Priority::None => 0,
        Priority::Low => 1,
        Priority::Medium => 2,
        Priority::High => 3,
    };
    let overdue = t.status().is_open() && t.due().is_some_and(|d| d < today);
    let project_id = t.project().map(|p| p.to_string());
    let project_name = t.project().and_then(|p| projects.get(&p).cloned());
    let prog = progress.get(&t.id()).copied().unwrap_or_default();
    TaskItem {
        id: t.id().to_string(),
        title: t.title().to_owned(),
        notes: t.notes().to_owned(),
        completed: !t.status().is_open(),
        priority,
        due_date: t.due().map(|d| d.to_string()),
        overdue,
        project_id,
        project_name,
        subtask_done: u32::try_from(prog.done).unwrap_or(u32::MAX),
        subtask_total: u32::try_from(prog.total).unwrap_or(u32::MAX),
    }
}

pub(crate) fn task_to_dto(t: &Task, today: Date) -> TaskItem {
    task_to_dto_rich(t, today, &HashMap::new(), &HashMap::new())
}

fn parse_task_id(id: &str) -> Result<TaskId, MobileError> {
    id.parse::<TaskId>().map_err(|_| MobileError::InvalidInput {
        reason: "Invalid task ID".into(),
    })
}

pub(crate) fn list_tasks(
    store: &Store,
    filter: MobileTaskFilter,
) -> Result<Vec<TaskItem>, MobileError> {
    let today = clock::today();
    let sf = match filter {
        MobileTaskFilter::Today => TaskFilter::Today(today),
        MobileTaskFilter::Upcoming => TaskFilter::Upcoming(today),
        MobileTaskFilter::All => TaskFilter::Open,
        MobileTaskFilter::Done => TaskFilter::Closed,
    };
    let tasks = store.list_tasks(sf)?;
    let projects: HashMap<ProjectId, String> = store
        .projects()?
        .into_iter()
        .map(|(p, _)| (p.id(), p.name().to_owned()))
        .collect();
    let progress = store.subtask_progress()?;
    Ok(tasks
        .iter()
        .map(|t| task_to_dto_rich(t, today, &projects, &progress))
        .collect())
}

pub(crate) fn quick_add_task(
    store: &mut Store,
    input: &str,
    default_today: bool,
) -> Result<TaskItem, MobileError> {
    let now = clock::now();
    let today = clock::today();
    let parsed = parse_quick_add(input, today);
    let mut task = Task::new(&parsed.title, now)?;
    if let Some(priority) = parsed.priority
        && priority != Priority::None
    {
        task.set_priority(priority, now)?;
    }
    let due = parsed.due.or_else(|| default_today.then_some(today));
    if due.is_some() {
        task.set_due(due, now)?;
    }
    if let Some(rule) = parsed.recurrence {
        task.set_recurrence(Some(rule), today, now)?;
    }
    store.save_task(&task)?;
    enqueue_task(store, &task);
    Ok(task_to_dto(&task, today))
}

pub(crate) fn toggle_task_done(store: &mut Store, id: &str) -> Result<TaskItem, MobileError> {
    let task_id = parse_task_id(id)?;
    let now = clock::now();
    let today = clock::today();
    let mut task = store.task(task_id)?.ok_or_else(|| MobileError::NotFound {
        reason: "Task not found".into(),
    })?;
    let completing = task.status().is_open();
    let next = if completing {
        TaskStatus::Done
    } else {
        TaskStatus::Todo
    };
    task.transition(next, now)?;
    if completing && let Some(next_task) = task.next_occurrence(today, now) {
        task.clear_recurrence_quietly();
        store.save_task(&next_task)?;
        enqueue_task(store, &next_task);
    }
    store.save_task(&task)?;
    enqueue_task(store, &task);
    Ok(task_to_dto(&task, today))
}

pub(crate) fn trash_task(store: &mut Store, id: &str) -> Result<(), MobileError> {
    let task_id = parse_task_id(id)?;
    let mut task = store.task(task_id)?.ok_or_else(|| MobileError::NotFound {
        reason: "Task not found".into(),
    })?;
    task.trash(clock::now());
    store.save_task(&task)?;
    enqueue_task(store, &task);
    Ok(())
}

pub(crate) fn restore_task(store: &mut Store, id: &str) -> Result<TaskItem, MobileError> {
    let task_id = parse_task_id(id)?;
    let today = clock::today();
    let mut task = store.task(task_id)?.ok_or_else(|| MobileError::NotFound {
        reason: "Task not found".into(),
    })?;
    task.restore(clock::now());
    store.save_task(&task)?;
    enqueue_task(store, &task);
    Ok(task_to_dto(&task, today))
}

pub(crate) fn update_task(
    store: &mut Store,
    id: &str,
    title: &str,
    notes: &str,
    priority: u8,
    due_date: Option<String>,
) -> Result<TaskItem, MobileError> {
    let task_id = parse_task_id(id)?;
    let now = clock::now();
    let today = clock::today();
    let mut task = store.task(task_id)?.ok_or_else(|| MobileError::NotFound {
        reason: "Task not found".into(),
    })?;

    let mut changed = false;
    if task.title() != title.trim() {
        task.rename(title, now)?;
        changed = true;
    }
    if task.notes() != notes {
        task.set_notes(notes, now)?;
        changed = true;
    }
    let parsed_priority = match priority {
        1 => Priority::Low,
        2 => Priority::Medium,
        3 => Priority::High,
        _ => Priority::None,
    };
    if task.priority() != parsed_priority {
        task.set_priority(parsed_priority, now)?;
        changed = true;
    }
    let parsed_due = match due_date.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(iso) => Some(iso.parse::<Date>().map_err(|_| MobileError::InvalidInput {
            reason: "Invalid due date".into(),
        })?),
        None => None,
    };
    if task.due() != parsed_due {
        task.set_due(parsed_due, now)?;
        changed = true;
    }
    if changed {
        store.save_task(&task)?;
        enqueue_task(store, &task);
    }
    Ok(task_to_dto(&task, today))
}

pub(crate) fn set_task_project(
    store: &mut Store,
    id: &str,
    project_id: Option<String>,
) -> Result<TaskItem, MobileError> {
    let task_id = parse_task_id(id)?;
    let now = clock::now();
    let today = clock::today();
    let mut task = store.task(task_id)?.ok_or_else(|| MobileError::NotFound {
        reason: "Task not found".into(),
    })?;
    let parsed_project = match project_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(raw) => Some(
            raw.parse::<ProjectId>()
                .map_err(|_| MobileError::InvalidInput {
                    reason: "Invalid project ID".into(),
                })?,
        ),
        None => None,
    };
    if task.project() != parsed_project {
        task.set_project(parsed_project, now)?;
        store.save_task(&task)?;
        enqueue_task(store, &task);
    }
    Ok(task_to_dto(&task, today))
}

pub(crate) fn list_projects(store: &Store) -> Result<Vec<ProjectItem>, MobileError> {
    let rows = store.projects()?;
    Ok(rows
        .into_iter()
        .map(|(p, open_count)| ProjectItem {
            id: p.id().to_string(),
            name: p.name().to_owned(),
            open_count: u32::try_from(open_count).unwrap_or(u32::MAX),
        })
        .collect())
}

pub(crate) fn create_project(store: &mut Store, name: &str) -> Result<ProjectItem, MobileError> {
    let now = clock::now();
    let project = Project::new(name, now)?;
    store.save_project(&project)?;
    if let Some(device_id) = sync::active_device_id(store) {
        let _ = enqueue_local_change(
            store,
            &device_id,
            Collection::Projects,
            &project.id().to_string(),
            &["record"],
            false,
            &SyncProjectPayload::from(&project),
            now,
        );
    }
    Ok(ProjectItem {
        id: project.id().to_string(),
        name: project.name().to_owned(),
        open_count: 0,
    })
}

pub(crate) fn list_subtasks(store: &Store, parent_id: &str) -> Result<Vec<TaskItem>, MobileError> {
    let pid = parse_task_id(parent_id)?;
    let today = clock::today();
    let subs = store.subtasks(pid)?;
    Ok(subs.iter().map(|t| task_to_dto(t, today)).collect())
}

pub(crate) fn add_subtask(
    store: &mut Store,
    parent_id: &str,
    title: &str,
) -> Result<TaskItem, MobileError> {
    let pid = parse_task_id(parent_id)?;
    let now = clock::now();
    let today = clock::today();
    let sub = Task::new_subtask(pid, title, now)?;
    store.save_task(&sub)?;
    enqueue_task(store, &sub);
    Ok(task_to_dto(&sub, today))
}

pub(crate) fn list_trashed_tasks(store: &Store) -> Result<Vec<TaskItem>, MobileError> {
    let today = clock::today();
    let tasks = store.list_tasks(TaskFilter::Trash)?;
    Ok(tasks.iter().map(|t| task_to_dto(t, today)).collect())
}

pub(crate) fn delete_task_permanently(store: &mut Store, id: &str) -> Result<(), MobileError> {
    let task_id = parse_task_id(id)?;
    store.delete_task_permanently(task_id)?;
    if let Some(device_id) = sync::active_device_id(store) {
        let _ = enqueue_local_change(
            store,
            &device_id,
            Collection::Tasks,
            &task_id.to_string(),
            &["deleted"],
            true,
            &serde_json::json!({}),
            clock::now(),
        );
    }
    Ok(())
}
