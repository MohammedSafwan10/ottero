//! Habits operations exposed to mobile clients (streaks, 7-day history, edit, archive, delete).

use std::collections::HashMap;

use jiff::civil::Date;
use ottero_core::{Habit, HabitId, Schedule, clock, parse_habit};
use ottero_storage::Store;
use ottero_sync_client::{enqueue_habit_change, enqueue_local_change};
use ottero_sync_protocol::Collection;

use crate::dto::HabitItem;
use crate::error::MobileError;
use crate::sync;

fn parse_habit_id(id: &str) -> Result<HabitId, MobileError> {
    id.parse::<HabitId>()
        .map_err(|_| MobileError::InvalidInput {
            reason: "Invalid habit ID".into(),
        })
}

fn habit_to_dto(h: &Habit, counts: &HashMap<Date, u16>, today: Date) -> HabitItem {
    let today_count = counts.get(&today).copied().unwrap_or(0);
    let target = h.target();
    let first = today
        .checked_sub(jiff::Span::new().days(90))
        .unwrap_or(today);
    let stats = h.stats(counts, first, today);
    let recent_days = (0..7)
        .rev()
        .map(|offset| {
            let day = today
                .checked_sub(jiff::Span::new().days(offset))
                .unwrap_or(today);
            counts.get(&day).copied().unwrap_or(0) >= target
        })
        .collect();
    HabitItem {
        id: h.id().to_string(),
        name: h.name().to_owned(),
        schedule_code: h.schedule().to_code(),
        target_per_day: target,
        today_count,
        due_today: h.schedule().is_due(today),
        done_today: today_count >= target,
        current_streak: u32::try_from(stats.current_streak).unwrap_or(u32::MAX),
        best_streak: u32::try_from(stats.best_streak).unwrap_or(u32::MAX),
        recent_days,
    }
}

pub(crate) fn list_habits(store: &Store) -> Result<Vec<HabitItem>, MobileError> {
    let today = clock::today();
    let habits = store.habits(false)?;
    let start = today
        .checked_sub(jiff::Span::new().days(90))
        .unwrap_or(today);
    let end = today.tomorrow().unwrap_or(today);
    let checkins = store.checkins(start..end)?;
    let empty = HashMap::new();
    Ok(habits
        .into_iter()
        .map(|h| {
            let counts = checkins.get(&h.id()).unwrap_or(&empty);
            habit_to_dto(&h, counts, today)
        })
        .collect())
}

pub(crate) fn quick_add_habit(store: &mut Store, input: &str) -> Result<HabitItem, MobileError> {
    let now = clock::now();
    let today = clock::today();
    let draft = parse_habit(input);
    let mut habit = Habit::new(&draft.name, draft.schedule, now)?;
    if draft.target > 1 {
        habit.set_target(draft.target, now)?;
    }
    store.save_habit(&habit)?;
    if let Some(device_id) = sync::active_device_id(store) {
        let _ = enqueue_habit_change(store, &device_id, &habit, now);
    }
    Ok(habit_to_dto(&habit, &HashMap::new(), today))
}

pub(crate) fn tap_habit_today(store: &mut Store, id: &str) -> Result<HabitItem, MobileError> {
    let habit_id = parse_habit_id(id)?;
    let today = clock::today();
    let habit = store
        .habit(habit_id)?
        .ok_or_else(|| MobileError::NotFound {
            reason: "Habit not found".into(),
        })?;
    let start = today
        .checked_sub(jiff::Span::new().days(90))
        .unwrap_or(today);
    let end = today.tomorrow().unwrap_or(today);
    let mut checkins = store.checkins(start..end)?;
    let counts = checkins.entry(habit_id).or_default();
    let current = counts.get(&today).copied().unwrap_or(0);
    let next = habit.tap(current, today, today)?;
    store.set_checkin(habit_id, today, next)?;
    counts.insert(today, next);
    Ok(habit_to_dto(&habit, counts, today))
}

pub(crate) fn update_habit(
    store: &mut Store,
    id: &str,
    name: &str,
    schedule_code: &str,
    target: u16,
) -> Result<HabitItem, MobileError> {
    let habit_id = parse_habit_id(id)?;
    let now = clock::now();
    let today = clock::today();
    let mut habit = store
        .habit(habit_id)?
        .ok_or_else(|| MobileError::NotFound {
            reason: "Habit not found".into(),
        })?;
    if habit.name() != name.trim() {
        habit.rename(name, now)?;
    }
    if let Some(schedule) = Schedule::from_code(schedule_code)
        && habit.schedule() != schedule
    {
        habit.set_schedule(schedule, now)?;
    }
    if habit.target() != target {
        habit.set_target(target.max(1), now)?;
    }
    store.save_habit(&habit)?;
    if let Some(device_id) = sync::active_device_id(store) {
        let _ = enqueue_habit_change(store, &device_id, &habit, now);
    }
    let start = today
        .checked_sub(jiff::Span::new().days(90))
        .unwrap_or(today);
    let checkins = store.checkins(start..today.tomorrow().unwrap_or(today))?;
    let empty = HashMap::new();
    Ok(habit_to_dto(
        &habit,
        checkins.get(&habit_id).unwrap_or(&empty),
        today,
    ))
}

pub(crate) fn archive_habit(store: &mut Store, id: &str) -> Result<(), MobileError> {
    let habit_id = parse_habit_id(id)?;
    let now = clock::now();
    let mut habit = store
        .habit(habit_id)?
        .ok_or_else(|| MobileError::NotFound {
            reason: "Habit not found".into(),
        })?;
    habit.archive(now)?;
    store.save_habit(&habit)?;
    if let Some(device_id) = sync::active_device_id(store) {
        let _ = enqueue_habit_change(store, &device_id, &habit, now);
    }
    Ok(())
}

pub(crate) fn delete_habit(store: &mut Store, id: &str) -> Result<(), MobileError> {
    let habit_id = parse_habit_id(id)?;
    store.delete_habit(habit_id)?;
    if let Some(device_id) = sync::active_device_id(store) {
        let _ = enqueue_local_change(
            store,
            &device_id,
            Collection::Habits,
            &habit_id.to_string(),
            &["record"],
            true,
            &serde_json::json!({}),
            clock::now(),
        );
    }
    Ok(())
}
