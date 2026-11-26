//! Tasks operations exposed to mobile clients.

use jiff::civil::Date;
use ottero_core::{Priority, Task, TaskId, TaskStatus, clock, parse_quick_add};
use ottero_storage::{Store, TaskFilter};

use crate::dto::{MobileTaskFilter, TaskItem};
use crate::error::MobileError;

pub(crate) fn task_to_dto(t: &Task, today: Date) -> TaskItem {
    let priority = match t.priority() {
        Priority::None => 0,
        Priority::Low => 1,
        Priority::Medium => 2,
        Priority::High => 3,
    };
    let overdue = t.status().is_open() && t.due().is_some_and(|d| d < today);
    TaskItem {
        id: t.id().to_string(),
        title: t.title().to_owned(),
        notes: t.notes().to_owned(),
        completed: !t.status().is_open(),
        priority,
        due_date: t.due().map(|d| d.to_string()),
        overdue,
    }
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
    Ok(tasks.iter().map(|t| task_to_dto(t, today)).collect())
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
    store.save_task(&task)?;
    Ok(task_to_dto(&task, today))
}

pub(crate) fn toggle_task_done(store: &mut Store, id: &str) -> Result<TaskItem, MobileError> {
    let task_id = parse_task_id(id)?;
    let now = clock::now();
    let today = clock::today();
    let mut task = store.task(task_id)?.ok_or_else(|| MobileError::NotFound {
        reason: "Task not found".into(),
    })?;
    let next = if task.status().is_open() {
        TaskStatus::Done
    } else {
        TaskStatus::Todo
    };
    task.transition(next, now)?;
    store.save_task(&task)?;
    Ok(task_to_dto(&task, today))
}

pub(crate) fn trash_task(store: &mut Store, id: &str) -> Result<(), MobileError> {
    let task_id = parse_task_id(id)?;
    let mut task = store.task(task_id)?.ok_or_else(|| MobileError::NotFound {
        reason: "Task not found".into(),
    })?;
    task.trash(clock::now());
    store.save_task(&task)?;
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
    Ok(task_to_dto(&task, today))
}
