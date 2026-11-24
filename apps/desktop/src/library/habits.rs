//! Habits, check-ins, and streak summaries on [`Library`].

use std::collections::HashMap;

use anyhow::{Context as _, Result};
use gpui_kit::Context;

use ottero_core::{Date, Habit, HabitId, Schedule, Stats, clock, parse_habit};

use super::{Library, LibraryEvent};

impl Library {
    // ---- Habits ----------------------------------------------------------

    /// Habits with today's state, the last 7 days and their statistics.
    pub fn habits(&self, archived: bool) -> Result<Vec<HabitSummary>> {
        let today = clock::today();
        let habits = self.store.habits(archived)?;
        let from = today
            .checked_add(jiff::Span::new().days(-ottero_core::habit::HISTORY_DAYS))
            .unwrap_or(today);
        let mut checkins = self.store.checkins(from..today.tomorrow()?)?;
        let firsts = self.store.first_checkins()?;
        let tz = jiff::tz::TimeZone::system();
        Ok(habits
            .into_iter()
            .map(|habit| {
                let counts = checkins.remove(&habit.id()).unwrap_or_default();
                let created = habit.created_at().to_zoned(tz.clone()).date();
                let first = firsts
                    .get(&habit.id())
                    .map_or(created, |d| (*d).min(created));
                let stats = habit.stats(&counts, first, today);
                HabitSummary {
                    today_count: counts.get(&today).copied().unwrap_or(0),
                    due_today: habit.schedule().is_due(today),
                    stats,
                    first,
                    counts,
                    habit,
                }
            })
            .collect())
    }

    pub fn habit(&self, id: HabitId) -> Option<HabitSummary> {
        let archived = self.store.habit(id).ok().flatten()?.is_archived();
        self.habits(archived)
            .ok()?
            .into_iter()
            .find(|h| h.habit.id() == id)
    }

    /// Creates a habit from quick-add text ("Run 3x a week").
    pub fn quick_add_habit(&mut self, input: &str, cx: &mut Context<Self>) -> Result<HabitId> {
        let draft = parse_habit(input);
        let now = clock::now();
        let mut habit = Habit::new(&draft.name, draft.schedule, now)?;
        habit.set_target(draft.target, now)?;
        self.store
            .save_habit(&habit)
            .context("Couldn't create the habit")?;
        self.record_sync(
            ottero_sync_protocol::Collection::Habits,
            &habit.id().to_string(),
            &["record"],
            false,
            &crate::sync::SyncHabitPayload::from(&habit),
        );
        cx.emit(LibraryEvent::HabitsChanged);
        Ok(habit.id())
    }

    /// One tap on a day: counts up to the goal, then clears.
    pub fn tap_habit(&mut self, id: HabitId, day: Date, cx: &mut Context<Self>) -> Result<u16> {
        let habit = self
            .store
            .habit(id)?
            .context("That habit no longer exists")?;
        let current = self
            .store
            .checkins(day..day.tomorrow()?)?
            .get(&id)
            .and_then(|c| c.get(&day).copied())
            .unwrap_or(0);
        let next = habit.tap(current, day, clock::today())?;
        self.set_habit_count(id, day, next, cx)?;
        Ok(next)
    }

    pub fn set_habit_count(
        &mut self,
        id: HabitId,
        day: Date,
        count: u16,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let habit = self
            .store
            .habit(id)?
            .context("That habit no longer exists")?;
        if habit.is_archived() {
            return Err(ottero_core::DomainError::Archived.into());
        }
        if day > clock::today() {
            return Err(ottero_core::DomainError::FutureCheckIn.into());
        }
        self.store
            .set_checkin(id, day, count.min(habit.target()))
            .context("Couldn't save the check-in")?;
        cx.emit(LibraryEvent::HabitsChanged);
        Ok(())
    }

    pub fn rename_habit(&mut self, id: HabitId, name: &str, cx: &mut Context<Self>) -> Result<()> {
        self.update_habit(id, cx, |h| h.rename(name, clock::now()))
    }

    pub fn set_habit_schedule(
        &mut self,
        id: HabitId,
        schedule: Schedule,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_habit(id, cx, |h| h.set_schedule(schedule, clock::now()))
    }

    pub fn set_habit_target(
        &mut self,
        id: HabitId,
        target: u16,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_habit(id, cx, |h| h.set_target(target, clock::now()))
    }

    pub fn archive_habit(&mut self, id: HabitId, cx: &mut Context<Self>) -> Result<()> {
        self.update_habit(id, cx, |h| h.archive(clock::now()))
    }

    pub fn unarchive_habit(&mut self, id: HabitId, cx: &mut Context<Self>) -> Result<()> {
        self.update_habit(id, cx, |h| {
            h.unarchive(clock::now());
            Ok(())
        })
    }

    /// Deletes a habit and its whole history (callers confirm first).
    pub fn delete_habit(&mut self, id: HabitId, cx: &mut Context<Self>) -> Result<()> {
        self.store.delete_habit(id)?;
        self.record_sync(
            ottero_sync_protocol::Collection::Habits,
            &id.to_string(),
            &["record"],
            true,
            &serde_json::json!({}),
        );
        cx.emit(LibraryEvent::HabitsChanged);
        Ok(())
    }

    pub fn move_habit(&mut self, id: HabitId, index: usize, cx: &mut Context<Self>) -> Result<()> {
        self.store
            .move_habit(id, index)
            .context("Couldn't reorder habits")?;
        cx.emit(LibraryEvent::HabitsChanged);
        Ok(())
    }

    fn update_habit(
        &mut self,
        id: HabitId,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Habit) -> ottero_core::DomainResult<()>,
    ) -> Result<()> {
        let mut habit = self
            .store
            .habit(id)?
            .context("That habit no longer exists")?;
        change(&mut habit)?;
        self.store
            .save_habit(&habit)
            .context("Couldn't save the habit")?;
        self.record_sync(
            ottero_sync_protocol::Collection::Habits,
            &habit.id().to_string(),
            &["record"],
            false,
            &crate::sync::SyncHabitPayload::from(&habit),
        );
        cx.emit(LibraryEvent::HabitsChanged);
        Ok(())
    }
}

/// A habit with what the screens show about it.
#[derive(Debug, Clone)]
pub struct HabitSummary {
    pub habit: Habit,
    pub today_count: u16,
    pub due_today: bool,
    pub stats: Stats,
    /// First day that counts (creation, or an earlier backfilled check-in).
    pub first: Date,
    /// Check-in counts for the history window.
    pub counts: HashMap<Date, u16>,
}

impl HabitSummary {
    #[must_use]
    pub fn done_today(&self) -> bool {
        self.today_count >= self.habit.target()
    }

    #[must_use]
    pub fn count_on(&self, day: Date) -> u16 {
        self.counts.get(&day).copied().unwrap_or(0)
    }

    /// Done this week (Monday-based), for "N times a week" habits.
    #[must_use]
    pub fn done_this_week(&self, today: Date) -> usize {
        let monday = today
            .checked_add(
                jiff::Span::new().days(-i64::from(today.weekday().to_monday_zero_offset())),
            )
            .unwrap_or(today);
        self.counts
            .iter()
            .filter(|(d, c)| **d >= monday && **d <= today && **c >= self.habit.target())
            .count()
    }
}
