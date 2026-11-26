//! Aggregated Today snapshot for the mobile home screen.

use ottero_core::clock;
use ottero_storage::{Store, TaskFilter};

use crate::dto::TodaySnapshot;
use crate::error::MobileError;
use crate::{calendar, habits, notes, tasks};

pub(crate) fn snapshot(store: &Store) -> Result<TodaySnapshot, MobileError> {
    let today = clock::today();
    let today_iso = today.to_string();
    let due_tasks = store
        .list_tasks(TaskFilter::Today(today))?
        .iter()
        .map(|t| tasks::task_to_dto(t, today))
        .collect();
    let tz = jiff::tz::TimeZone::system();
    let day_start = today
        .at(0, 0, 0, 0)
        .to_zoned(tz.clone())
        .map_or_else(|_| clock::now(), |z| z.timestamp());
    let day_end = today
        .tomorrow()
        .unwrap_or(today)
        .at(0, 0, 0, 0)
        .to_zoned(tz)
        .map_or_else(|_| clock::now(), |z| z.timestamp());
    let counts = store.task_counts(today, day_start, day_end)?;
    let habits = habits::list_habits(store)?;
    let today_events = calendar::list_events_in_range(store, &today_iso, &today_iso)?;
    let mut recent_notes: Vec<_> = store
        .list_notes(false)?
        .into_iter()
        .map(notes::summary_to_dto)
        .collect();
    recent_notes.truncate(5);

    Ok(TodaySnapshot {
        date_iso: today_iso,
        due_tasks,
        habits,
        recent_notes,
        today_events,
        completed_today_count: counts.completed_today as u32,
        overdue_count: counts.overdue as u32,
    })
}
