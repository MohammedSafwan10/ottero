//! Calendar events and the tasks a calendar shows by due date.

use std::ops::Range;

use ottero_core::event::{exceptions_from_code, exceptions_to_code};
use ottero_core::{Date, DateTime, Event, EventId, EventRecord, Recurrence, Task, Time};
use rusqlite::{OptionalExtension, Row, params};

use crate::codec::{date_to_sql, opt_ts_from_sql, ts_from_sql, ts_to_sql, uuid_from_sql};
use crate::{StorageError, StorageResult, Store};

const COLUMNS: &str = "id, title, notes, location, all_day, start_at, end_at, recurrence, created_at, updated_at, deleted_at, exceptions, until, occasion";

/// Fixed-width ISO text, so SQLite's string order is time order.
fn dt_to_sql(dt: DateTime) -> String {
    dt.strftime("%Y-%m-%dT%H:%M:%S").to_string()
}

fn dt_from_sql(s: &str, column: &'static str) -> StorageResult<DateTime> {
    s.parse().map_err(|_| StorageError::Corrupt(column))
}

impl Store {
    pub fn save_event(&mut self, event: &Event) -> StorageResult<()> {
        let r = event.to_record();
        self.conn.execute(
            "INSERT INTO events (id, title, notes, location, all_day, start_at, end_at, recurrence, created_at, updated_at, deleted_at, exceptions, until, occasion)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT (id) DO UPDATE SET
                title = excluded.title,
                notes = excluded.notes,
                location = excluded.location,
                all_day = excluded.all_day,
                start_at = excluded.start_at,
                end_at = excluded.end_at,
                recurrence = excluded.recurrence,
                updated_at = excluded.updated_at,
                deleted_at = excluded.deleted_at,
                exceptions = excluded.exceptions,
                until = excluded.until,
                occasion = excluded.occasion",
            params![
                r.id.to_string(),
                r.title,
                r.notes,
                r.location,
                r.all_day,
                dt_to_sql(r.start),
                dt_to_sql(r.end),
                r.recurrence.map(Recurrence::to_code),
                ts_to_sql(r.created_at),
                ts_to_sql(r.updated_at),
                r.deleted_at.map(ts_to_sql),
                exceptions_to_code(&r.exceptions),
                r.until.map(date_to_sql),
                r.occasion.map(ottero_core::Occasion::to_code),
            ],
        )?;
        Ok(())
    }

    pub fn event(&self, id: EventId) -> StorageResult<Option<Event>> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM events WHERE id = ?1"),
                [id.to_string()],
                raw_event,
            )
            .optional()?
            .map(RawEvent::into_event)
            .transpose()
    }

    /// Live events that may touch a day in `days` (end exclusive): one-off
    /// events overlapping the range, repeating events that began before its
    /// end, and repeating events with exceptions (a moved occurrence can land
    /// before the series starts). Expand with [`Event::occurrences`]. Ordered by start.
    pub fn events_between(&self, days: Range<Date>) -> StorageResult<Vec<Event>> {
        let from = dt_to_sql(days.start.to_datetime(Time::midnight()));
        let to = dt_to_sql(days.end.to_datetime(Time::midnight()));
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {COLUMNS} FROM events
             WHERE deleted_at IS NULL
               AND ((start_at < ?2 AND (recurrence IS NOT NULL OR end_at > ?1))
                    OR exceptions IS NOT NULL)
             ORDER BY start_at, id"
        ))?;
        let rows = stmt.query_map([from, to], raw_event)?.collect::<Vec<_>>();
        rows.into_iter()
            .map(|row| row.map_err(StorageError::from)?.into_event())
            .collect()
    }

    /// Every event (active and trashed), for full sync snapshots.
    pub fn all_events(&self) -> StorageResult<Vec<Event>> {
        let mut stmt = self
            .conn
            .prepare_cached(&format!("SELECT {COLUMNS} FROM events"))?;
        let rows = stmt.query_map([], raw_event)?.collect::<Vec<_>>();
        rows.into_iter()
            .map(|row| row.map_err(StorageError::from)?.into_event())
            .collect()
    }

    /// Trashed events, most recently trashed first.
    pub fn trashed_events(&self) -> StorageResult<Vec<Event>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {COLUMNS} FROM events WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC"
        ))?;
        let rows = stmt.query_map([], raw_event)?.collect::<Vec<_>>();
        rows.into_iter()
            .map(|row| row.map_err(StorageError::from)?.into_event())
            .collect()
    }

    pub fn delete_event_permanently(&mut self, id: EventId) -> StorageResult<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM events WHERE id = ?1", [id.to_string()])?
            > 0)
    }

    /// Top-level, non-trashed tasks (open or done) due on a day in `days`,
    /// by due date then priority. Feeds the calendar.
    pub fn tasks_due_between(&self, days: Range<Date>) -> StorageResult<Vec<Task>> {
        let ids: Vec<String> = {
            let mut stmt = self.conn.prepare_cached(
                "SELECT id FROM tasks
                 WHERE deleted_at IS NULL AND parent_id IS NULL AND status IN ('todo', 'in_progress', 'done')
                   AND due >= ?1 AND due < ?2
                 ORDER BY due, status = 'done', priority DESC, created_at",
            )?;
            stmt.query_map([date_to_sql(days.start), date_to_sql(days.end)], |r| {
                r.get(0)
            })?
            .collect::<Result<_, _>>()?
        };
        let mut tasks = Vec::with_capacity(ids.len());
        for id in ids {
            let id = ottero_core::TaskId::from_uuid(uuid_from_sql(&id, "tasks.id")?);
            if let Some(task) = self.task(id)? {
                tasks.push(task);
            }
        }
        Ok(tasks)
    }
}

struct RawEvent {
    id: String,
    title: String,
    notes: String,
    location: String,
    all_day: bool,
    start: String,
    end: String,
    recurrence: Option<String>,
    created_at: i64,
    updated_at: i64,
    deleted_at: Option<i64>,
    exceptions: Option<String>,
    until: Option<String>,
    occasion: Option<String>,
}

impl RawEvent {
    fn into_event(self) -> StorageResult<Event> {
        Ok(Event::from_record(EventRecord {
            id: EventId::from_uuid(uuid_from_sql(&self.id, "events.id")?),
            title: self.title,
            notes: self.notes,
            location: self.location,
            all_day: self.all_day,
            start: dt_from_sql(&self.start, "events.start_at")?,
            end: dt_from_sql(&self.end, "events.end_at")?,
            // An unknown rule (e.g. from a newer version) is dropped, not fatal.
            recurrence: self.recurrence.as_deref().and_then(Recurrence::from_code),
            created_at: ts_from_sql(self.created_at, "events.created_at")?,
            updated_at: ts_from_sql(self.updated_at, "events.updated_at")?,
            deleted_at: opt_ts_from_sql(self.deleted_at, "events.deleted_at")?,
            exceptions: self
                .exceptions
                .as_deref()
                .map(exceptions_from_code)
                .unwrap_or_default(),
            // Like an unknown rule: an unreadable end repeats forever.
            until: self.until.as_deref().and_then(|d| d.parse().ok()),
            occasion: self
                .occasion
                .as_deref()
                .and_then(ottero_core::Occasion::from_code),
        }))
    }
}

fn raw_event(row: &Row<'_>) -> rusqlite::Result<RawEvent> {
    Ok(RawEvent {
        id: row.get(0)?,
        title: row.get(1)?,
        notes: row.get(2)?,
        location: row.get(3)?,
        all_day: row.get(4)?,
        start: row.get(5)?,
        end: row.get(6)?,
        recurrence: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        deleted_at: row.get(10)?,
        exceptions: row.get(11)?,
        until: row.get(12)?,
        occasion: row.get(13)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ottero_core::Timestamp;

    fn at(secs: i64) -> Timestamp {
        Timestamp::from_second(secs).expect("ts")
    }
    fn d(s: &str) -> Date {
        s.parse().expect("date")
    }
    fn dt(s: &str) -> DateTime {
        s.parse().expect("datetime")
    }
    fn titles(events: &[Event]) -> Vec<&str> {
        events.iter().map(Event::title).collect()
    }

    #[test]
    fn exceptions_persist_and_moved_occurrences_are_found() {
        let mut store = Store::open_in_memory().expect("store");
        let mut e = Event::new_timed(
            "Review",
            dt("2026-11-10T15:00"),
            dt("2026-11-10T16:00"),
            at(1),
        )
        .expect("event");
        e.set_recurrence(Some(Recurrence::WEEKLY), at(1))
            .expect("repeat");
        e.skip_occurrence(d("2026-11-17"), at(2)).expect("skip");
        // The first one moves a week earlier than the series starts.
        e.move_occurrence(
            d("2026-11-10"),
            dt("2026-11-03T15:00"),
            dt("2026-11-03T16:00"),
            at(2),
        )
        .expect("move");
        e.set_until(Some(d("2027-01-31")), at(3)).expect("end");
        store.save_event(&e).expect("save");
        assert_eq!(store.event(e.id()).expect("load").as_ref(), Some(&e));
        let early = store
            .events_between(d("2026-11-02")..d("2026-11-09"))
            .expect("range");
        assert_eq!(titles(&early), ["Review"]);
        assert_eq!(
            early[0]
                .occurrences(d("2026-11-02")..d("2026-11-09"), 10)
                .len(),
            1
        );
    }

    #[test]
    fn occasions_persist() {
        let mut store = Store::open_in_memory().expect("store");
        let mut e = Event::new_all_day("Mom's birthday", d("2026-11-20"), d("2026-11-20"), at(1))
            .expect("event");
        e.set_occasion(
            Some(ottero_core::Occasion {
                kind: ottero_core::OccasionKind::Birthday,
                year: Some(1966),
            }),
            at(1),
        )
        .expect("occasion");
        store.save_event(&e).expect("save");
        assert_eq!(store.event(e.id()).expect("load"), Some(e));
    }

    #[test]
    fn events_round_trip_including_updates() {
        let mut store = Store::open_in_memory().expect("store");
        let mut e = Event::new_timed(
            "Dentist",
            dt("2026-11-10T15:00"),
            dt("2026-11-10T16:30"),
            at(1),
        )
        .expect("event");
        e.set_location("Main St", at(1)).expect("loc");
        e.set_recurrence(Some(Recurrence::MONTHLY), at(1))
            .expect("repeat");
        store.save_event(&e).expect("save");
        assert_eq!(store.event(e.id()).expect("load").as_ref(), Some(&e));

        e.set_all_day_dates(d("2026-11-11"), d("2026-11-12"), at(2))
            .expect("all day");
        store.save_event(&e).expect("save");
        assert_eq!(store.event(e.id()).expect("load"), Some(e));
    }

    #[test]
    fn range_query_finds_overlaps_and_repeats_only() {
        let mut store = Store::open_in_memory().expect("store");
        let timed = |title: &str, start: &str, end: &str| {
            Event::new_timed(title, dt(start), dt(end), at(1)).expect("event")
        };
        let before = timed("Before", "2026-11-01T10:00", "2026-11-01T11:00");
        let ends_at_range_start = timed("Edge", "2026-11-08T23:00", "2026-11-09T00:00");
        let overnight_in = timed("Overnight", "2026-11-08T22:00", "2026-11-09T02:00");
        let inside = timed("Inside", "2026-11-10T09:00", "2026-11-10T10:00");
        let after = timed("After", "2026-11-16T00:00", "2026-11-16T01:00");
        let trip =
            Event::new_all_day("Trip", d("2026-11-05"), d("2026-11-09"), at(1)).expect("trip");
        let mut standup = timed("Standup", "2026-10-01T09:30", "2026-10-01T09:45");
        standup
            .set_recurrence(Some(Recurrence::WORKDAYS), at(1))
            .expect("repeat");
        let mut trashed = timed("Trashed", "2026-11-10T12:00", "2026-11-10T13:00");
        trashed.trash(at(2)).expect("trash");
        for e in [
            &before,
            &ends_at_range_start,
            &overnight_in,
            &inside,
            &after,
            &trip,
            &standup,
            &trashed,
        ] {
            store.save_event(e).expect("save");
        }

        let week = d("2026-11-09")..d("2026-11-16");
        let found = store.events_between(week.clone()).expect("query");
        assert_eq!(titles(&found), ["Standup", "Trip", "Overnight", "Inside"]);
        // Expansion is the domain's job; the repeat yields Mon–Fri.
        let standups = found[0].occurrences(week, 100);
        assert_eq!(standups.len(), 5);

        assert_eq!(titles(&store.trashed_events().expect("trash")), ["Trashed"]);
        assert!(
            store
                .delete_event_permanently(trashed.id())
                .expect("delete")
        );
        assert!(store.event(trashed.id()).expect("load").is_none());
    }

    #[test]
    fn database_rejects_inverted_ranges() {
        let store = Store::open_in_memory().expect("store");
        let result = store.conn.execute(
            "INSERT INTO events (id, title, all_day, start_at, end_at, created_at, updated_at)
             VALUES ('x', 'y', 0, '2026-11-10T10:00:00', '2026-11-10T09:00:00', 0, 0)",
            [],
        );
        assert!(result.is_err(), "CHECK constraint must reject end <= start");
    }

    #[test]
    fn corrupt_times_surface_as_errors() {
        let store = Store::open_in_memory().expect("store");
        store
            .conn
            .execute(
                "INSERT INTO events (id, title, all_day, start_at, end_at, created_at, updated_at)
                 VALUES (?1, 'y', 0, '2026-11-10T25:00:00', '2026-11-10T26:00:00', 0, 0)",
                [EventId::new().to_string()],
            )
            .expect("insert");
        assert!(matches!(
            store.events_between(d("2000-01-01")..d("2100-01-01")),
            Err(StorageError::Corrupt(_))
        ));
    }

    #[test]
    fn tasks_due_between_includes_done_but_not_trash_or_subtasks() {
        let mut store = Store::open_in_memory().expect("store");
        let mk = |title: &str, due: &str| {
            let mut t = Task::new(title, at(1)).expect("task");
            t.set_due(Some(d(due)), at(1)).expect("due");
            t
        };
        let open = mk("Open", "2026-11-10");
        let mut done = mk("Done", "2026-11-10");
        done.complete(at(2)).expect("done");
        let mut trashed = mk("Trashed", "2026-11-10");
        trashed.trash(at(2));
        let outside = mk("Outside", "2026-11-20");
        let mut sub = Task::new_subtask(open.id(), "Sub", at(1)).expect("sub");
        sub.set_due(Some(d("2026-11-10")), at(1)).expect("due");
        store
            .save_tasks(&[&open, &done, &trashed, &outside, &sub])
            .expect("save");
        let found = store
            .tasks_due_between(d("2026-11-09")..d("2026-11-16"))
            .expect("query");
        let names: Vec<&str> = found.iter().map(Task::title).collect();
        assert_eq!(names, ["Open", "Done"]);
    }
}
