//! Habits operations exposed to mobile clients.

use ottero_core::{Habit, HabitId, clock, parse_habit};
use ottero_storage::Store;

use crate::dto::HabitItem;
use crate::error::MobileError;

fn parse_habit_id(id: &str) -> Result<HabitId, MobileError> {
    id.parse::<HabitId>()
        .map_err(|_| MobileError::InvalidInput {
            reason: "Invalid habit ID".into(),
        })
}

pub(crate) fn list_habits(store: &Store) -> Result<Vec<HabitItem>, MobileError> {
    let today = clock::today();
    let habits = store.habits(false)?;
    let checkins = store.checkins(today..today.tomorrow().unwrap_or(today))?;
    Ok(habits
        .into_iter()
        .map(|h| {
            let today_count = checkins
                .get(&h.id())
                .and_then(|m| m.get(&today))
                .copied()
                .unwrap_or(0);
            let due_today = h.schedule().is_due(today);
            let target = h.target();
            HabitItem {
                id: h.id().to_string(),
                name: h.name().to_owned(),
                target_per_day: target,
                today_count,
                due_today,
                done_today: today_count >= target,
            }
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
    Ok(HabitItem {
        id: habit.id().to_string(),
        name: habit.name().to_owned(),
        target_per_day: habit.target(),
        today_count: 0,
        due_today: habit.schedule().is_due(today),
        done_today: false,
    })
}

pub(crate) fn tap_habit_today(store: &mut Store, id: &str) -> Result<HabitItem, MobileError> {
    let habit_id = parse_habit_id(id)?;
    let today = clock::today();
    let habit = store
        .habit(habit_id)?
        .ok_or_else(|| MobileError::NotFound {
            reason: "Habit not found".into(),
        })?;
    let checkins = store.checkins(today..today.tomorrow().unwrap_or(today))?;
    let current = checkins
        .get(&habit_id)
        .and_then(|m| m.get(&today))
        .copied()
        .unwrap_or(0);
    let next = habit.tap(current, today, today)?;
    store.set_checkin(habit_id, today, next)?;
    let target = habit.target();
    Ok(HabitItem {
        id: habit.id().to_string(),
        name: habit.name().to_owned(),
        target_per_day: target,
        today_count: next,
        due_today: habit.schedule().is_due(today),
        done_today: next >= target,
    })
}
