//! Calendar events, recurrence expansion, and `.ics` import on [`Library`].

use std::collections::HashMap;
use std::ops::Range;

use anyhow::{Context as _, Result};
use gpui_kit::Context;

use ottero_core::{
    Date, DateTime, Event, EventId, Exception, Occurrence, Recurrence, Task, clock, parse_event,
};

use super::{Library, LibraryEvent};

impl Library {
    // ---- Calendar --------------------------------------------------------

    /// Everything a calendar shows for `days` (end exclusive): event
    /// occurrences in time order (all-day first within a day) and tasks due
    /// in the range.
    pub fn calendar(&self, days: Range<Date>) -> Result<CalendarSpan> {
        let events = self.store.events_between(days.clone())?;
        let mut occurrences: Vec<Occurrence> = events
            .iter()
            .flat_map(|e| e.occurrences(days.clone(), MAX_OCCURRENCES_PER_EVENT))
            .collect();
        occurrences.sort_by_key(|o| (o.start.date(), !o.all_day, o.start, o.end));
        let tasks = self.store.tasks_due_between(days.clone())?;
        Ok(CalendarSpan {
            days,
            occurrences,
            events: events.into_iter().map(|e| (e.id(), e)).collect(),
            tasks,
        })
    }

    /// One event by ID, or `None` if it was deleted (or can't be read).
    pub fn event(&self, id: EventId) -> Option<Event> {
        self.store.event(id).ok().flatten()
    }

    /// Creates an event from quick-add text ("Lunch with Sam fri 1pm").
    /// Text without a day lands on `default_day` (the selected day).
    pub fn quick_add_event(
        &mut self,
        input: &str,
        default_day: Date,
        cx: &mut Context<Self>,
    ) -> Result<EventId> {
        let draft = parse_event(input, clock::today()).on_default_day(default_day);
        let now = clock::now();
        let mut event = if draft.all_day {
            Event::new_all_day(&draft.title, draft.start.date(), draft.last_day(), now)?
        } else {
            Event::new_timed(&draft.title, draft.start, draft.end, now)?
        };
        event.set_recurrence(draft.recurrence, now)?;
        event.set_occasion(draft.occasion, now)?;
        self.save_new_event(&event, cx)
    }

    /// A timed event (double-click on the time grid).
    pub fn create_timed_event(
        &mut self,
        title: &str,
        start: DateTime,
        end: DateTime,
        cx: &mut Context<Self>,
    ) -> Result<EventId> {
        let event = Event::new_timed(title, start, end, clock::now())?;
        self.save_new_event(&event, cx)
    }

    /// An all-day event on one day (double-click on a month day).
    pub fn create_all_day_event(
        &mut self,
        title: &str,
        day: Date,
        cx: &mut Context<Self>,
    ) -> Result<EventId> {
        let event = Event::new_all_day(title, day, day, clock::now())?;
        self.save_new_event(&event, cx)
    }

    /// Imports an iCalendar file's text. Events already in the calendar
    /// (same title, times and repeat) are skipped, so importing the same
    /// file twice adds nothing.
    pub fn import_ics(&mut self, text: &str, cx: &mut Context<Self>) -> Result<IcsReport> {
        let local = jiff::tz::TimeZone::system();
        let zone = |name: &str| jiff::tz::db().get(name).ok();
        let parsed = ottero_core::ics::parse_ics(text, &local, &zone, clock::now())
            .map_err(|msg| anyhow::anyhow!(msg))?;
        let mut report = IcsReport {
            skipped: parsed.skipped,
            simplified: parsed.simplified,
            ..IcsReport::default()
        };
        for event in parsed.events {
            let day = event.start().date();
            let next = day.tomorrow().unwrap_or(day);
            let exists = self.store.events_between(day..next)?.iter().any(|e| {
                e.title() == event.title()
                    && e.start() == event.start()
                    && e.end() == event.end()
                    && e.recurrence() == event.recurrence()
            });
            if exists {
                report.duplicates += 1;
                continue;
            }
            self.store
                .save_event(&event)
                .context("Couldn't save the imported events")?;
            self.record_sync(
                ottero_sync_protocol::Collection::Events,
                &event.id().to_string(),
                &["record"],
                false,
                &crate::sync::SyncEventPayload::from(&event),
            );
            report.added += 1;
        }
        cx.emit(LibraryEvent::EventsChanged);
        Ok(report)
    }

    fn save_new_event(&mut self, event: &Event, cx: &mut Context<Self>) -> Result<EventId> {
        self.store
            .save_event(event)
            .context("Couldn't create the event")?;
        self.record_sync(
            ottero_sync_protocol::Collection::Events,
            &event.id().to_string(),
            &["record"],
            false,
            &crate::sync::SyncEventPayload::from(event),
        );
        cx.emit(LibraryEvent::EventsChanged);
        Ok(event.id())
    }

    pub fn rename_event(&mut self, id: EventId, title: &str, cx: &mut Context<Self>) -> Result<()> {
        self.update_event(id, cx, |e| e.rename(title, clock::now()))
    }

    pub fn set_event_notes(
        &mut self,
        id: EventId,
        notes: &str,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_event(id, cx, |e| e.set_notes(notes, clock::now()))
    }

    pub fn set_event_location(
        &mut self,
        id: EventId,
        location: &str,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_event(id, cx, |e| e.set_location(location, clock::now()))
    }

    /// Makes the event timed, `start..end`.
    pub fn set_event_times(
        &mut self,
        id: EventId,
        start: DateTime,
        end: DateTime,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_event(id, cx, |e| e.set_times(start, end, clock::now()))
    }

    /// Makes the event all-day on `first..=last`.
    pub fn set_event_all_day(
        &mut self,
        id: EventId,
        first: Date,
        last: Date,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_event(id, cx, |e| e.set_all_day_dates(first, last, clock::now()))
    }

    /// Moves the event to start on `day`, keeping its times and length.
    pub fn move_event(&mut self, id: EventId, day: Date, cx: &mut Context<Self>) -> Result<()> {
        self.update_event(id, cx, |e| e.move_to_date(day, clock::now()))
    }

    pub fn set_event_recurrence(
        &mut self,
        id: EventId,
        rule: Option<Recurrence>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_event(id, cx, |e| e.set_recurrence(rule, clock::now()))
    }

    /// Makes the event a birthday or anniversary (all-day, yearly), or a
    /// plain event again.
    pub fn set_event_occasion(
        &mut self,
        id: EventId,
        occasion: Option<ottero_core::Occasion>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_event(id, cx, |e| e.set_occasion(occasion, clock::now()))
    }

    /// Ends a repeating event after `until` (`None` = repeat forever).
    pub fn set_event_until(
        &mut self,
        id: EventId,
        until: Option<Date>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_event(id, cx, |e| e.set_until(until, clock::now()))
    }

    pub fn trash_event(&mut self, id: EventId, cx: &mut Context<Self>) -> Result<()> {
        self.update_event(id, cx, |e| e.trash(clock::now()))
    }

    /// "Delete this event only" on a repeating event. Returns what to pass
    /// to [`Library::restore_occurrence`] for Undo.
    pub fn skip_occurrence(
        &mut self,
        id: EventId,
        series_date: Date,
        cx: &mut Context<Self>,
    ) -> Result<Option<Exception>> {
        let mut previous = None;
        self.update_event(id, cx, |e| {
            previous = e.skip_occurrence(series_date, clock::now())?;
            Ok(())
        })?;
        Ok(previous)
    }

    /// "Change this event only": the occurrence on `series_date` happens
    /// `start..end` instead. Returns what Undo needs.
    pub fn move_occurrence(
        &mut self,
        id: EventId,
        series_date: Date,
        start: DateTime,
        end: DateTime,
        cx: &mut Context<Self>,
    ) -> Result<Option<Exception>> {
        let mut previous = None;
        self.update_event(id, cx, |e| {
            previous = e.move_occurrence(series_date, start, end, clock::now())?;
            Ok(())
        })?;
        Ok(previous)
    }

    /// Moves one occurrence to `day`, keeping its times and length (a drag
    /// onto a month or all-day cell). Returns what Undo needs.
    pub fn move_occurrence_to_day(
        &mut self,
        id: EventId,
        series_date: Date,
        day: Date,
        cx: &mut Context<Self>,
    ) -> Result<Option<Exception>> {
        let occ = self
            .event(id)
            .and_then(|e| e.occurrence_on(series_date))
            .context("That event no longer exists")?;
        let shift = day - occ.start.date();
        let start = occ.start.checked_add(shift)?;
        let end = occ.end.checked_add(shift)?;
        self.move_occurrence(id, series_date, start, end, cx)
    }

    /// Undo for [`Library::skip_occurrence`] and [`Library::move_occurrence`].
    pub fn restore_occurrence(
        &mut self,
        id: EventId,
        series_date: Date,
        previous: Option<Exception>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_event(id, cx, |e| {
            e.restore_exception(series_date, previous, clock::now())
        })
    }

    /// Deleted events, most recently deleted first.
    pub fn deleted_events(&self) -> Result<Vec<Event>> {
        Ok(self.store.trashed_events()?)
    }

    /// Permanently removes a deleted event (callers confirm first).
    pub fn delete_event_forever(&mut self, id: EventId, cx: &mut Context<Self>) -> Result<()> {
        let event = self
            .store
            .event(id)?
            .context("That event no longer exists")?;
        if !event.is_trashed() {
            anyhow::bail!("Only deleted events can be removed for good.");
        }
        self.store
            .delete_event_permanently(id)
            .context("Couldn't delete the event")?;
        self.record_sync(
            ottero_sync_protocol::Collection::Events,
            &id.to_string(),
            &["record"],
            true,
            &serde_json::json!({}),
        );
        cx.emit(LibraryEvent::EventsChanged);
        Ok(())
    }

    /// Permanently removes every deleted event. Returns how many.
    pub fn empty_event_trash(&mut self, cx: &mut Context<Self>) -> Result<usize> {
        let trashed = self.store.trashed_events()?;
        for event in &trashed {
            self.store
                .delete_event_permanently(event.id())
                .context("Couldn't empty the trash")?;
        }
        cx.emit(LibraryEvent::EventsChanged);
        Ok(trashed.len())
    }

    pub fn restore_event(&mut self, id: EventId, cx: &mut Context<Self>) -> Result<()> {
        let mut event = self
            .store
            .event(id)?
            .context("That event no longer exists")?;
        event.restore(clock::now());
        self.store
            .save_event(&event)
            .context("Couldn't save the event")?;
        self.record_sync(
            ottero_sync_protocol::Collection::Events,
            &event.id().to_string(),
            &["record"],
            false,
            &crate::sync::SyncEventPayload::from(&event),
        );
        cx.emit(LibraryEvent::EventsChanged);
        Ok(())
    }

    fn update_event(
        &mut self,
        id: EventId,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Event) -> ottero_core::DomainResult<()>,
    ) -> Result<()> {
        let mut event = self
            .store
            .event(id)?
            .context("That event no longer exists")?;
        change(&mut event)?;
        self.store
            .save_event(&event)
            .context("Couldn't save the event")?;
        self.record_sync(
            ottero_sync_protocol::Collection::Events,
            &event.id().to_string(),
            &["record"],
            false,
            &crate::sync::SyncEventPayload::from(&event),
        );
        cx.emit(LibraryEvent::EventsChanged);
        Ok(())
    }
}

/// Repeats expanded per event and range; bounds work for daily repeats over a long agenda.
const MAX_OCCURRENCES_PER_EVENT: usize = 500;

/// What an `.ics` import did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IcsReport {
    pub added: usize,
    /// Already in the calendar.
    pub duplicates: usize,
    /// Unreadable in the file.
    pub skipped: usize,
    /// Repeat rule not supported; first date only.
    pub simplified: usize,
}

impl IcsReport {
    /// "Imported 12 events. 3 were already in your calendar."
    #[must_use]
    pub fn message(&self) -> String {
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        let mut parts = vec![if self.added == 0 {
            "Nothing new to import.".to_owned()
        } else {
            format!("Imported {} event{}.", self.added, plural(self.added))
        }];
        if self.duplicates > 0 {
            parts.push(format!(
                "{} {} already in your calendar.",
                self.duplicates,
                if self.duplicates == 1 { "was" } else { "were" }
            ));
        }
        if self.simplified > 0 {
            parts.push(format!(
                "{} repeating event{} used a rule Ottero can't follow yet, so only the first date was added.",
                self.simplified,
                plural(self.simplified)
            ));
        }
        if self.skipped > 0 {
            parts.push(format!(
                "{} event{} couldn't be read.",
                self.skipped,
                plural(self.skipped)
            ));
        }
        parts.join(" ")
    }
}

/// What a calendar range shows.
#[derive(Debug, Clone, Default)]
pub struct CalendarSpan {
    pub days: Range<Date>,
    pub occurrences: Vec<Occurrence>,
    pub events: HashMap<EventId, Event>,
    pub tasks: Vec<Task>,
}

impl CalendarSpan {
    /// Occurrences that touch `day`, in display order.
    pub fn on_day(&self, day: Date) -> impl Iterator<Item = &Occurrence> {
        self.occurrences
            .iter()
            .filter(move |o| o.days().contains(&day))
    }

    /// Tasks due on `day`.
    pub fn tasks_on(&self, day: Date) -> impl Iterator<Item = &Task> {
        self.tasks.iter().filter(move |t| t.due() == Some(day))
    }
}
