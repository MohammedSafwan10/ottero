//! Calendar event operations exposed to mobile clients.

use jiff::civil::{Date, Time};
use ottero_core::{Event, EventId, clock, holidays, parse_event};
use ottero_storage::Store;
use ottero_sync_client::{enqueue_event_change, enqueue_local_change};
use ottero_sync_protocol::Collection;

use crate::dto::{CalendarEventItem, HolidayItem};
use crate::error::MobileError;
use crate::sync;

fn enqueue_event(store: &Store, event: &Event) {
    if let Some(device_id) = sync::active_device_id(store) {
        let _ = enqueue_event_change(store, &device_id, event, clock::now());
    }
}

fn event_to_dto(event: &Event) -> CalendarEventItem {
    CalendarEventItem {
        id: event.id().to_string(),
        title: event.title().to_owned(),
        notes: event.notes().to_owned(),
        location: event.location().to_owned(),
        date_iso: event.start().date().to_string(),
        start_time: (!event.is_all_day()).then(|| format_time(event.start().time())),
        end_time: (!event.is_all_day()).then(|| format_time(event.end().time())),
        all_day: event.is_all_day(),
    }
}

fn parse_hhmm(raw: &str) -> Result<Time, MobileError> {
    let trimmed = raw.trim();
    let (h, m) = trimmed
        .split_once(':')
        .ok_or_else(|| MobileError::InvalidInput {
            reason: "Time must be HH:MM".into(),
        })?;
    let hour: i8 = h.parse().map_err(|_| MobileError::InvalidInput {
        reason: "Invalid hour".into(),
    })?;
    let minute: i8 = m.parse().map_err(|_| MobileError::InvalidInput {
        reason: "Invalid minute".into(),
    })?;
    Time::new(hour, minute, 0, 0).map_err(|_| MobileError::InvalidInput {
        reason: "Invalid time".into(),
    })
}

fn parse_event_id(id: &str) -> Result<EventId, MobileError> {
    id.parse::<EventId>()
        .map_err(|_| MobileError::InvalidInput {
            reason: "Invalid event ID".into(),
        })
}

fn parse_iso_date(iso: &str) -> Result<Date, MobileError> {
    iso.parse::<Date>().map_err(|_| MobileError::InvalidInput {
        reason: "Invalid date".into(),
    })
}

fn format_time(t: Time) -> String {
    format!("{:02}:{:02}", t.hour(), t.minute())
}

pub(crate) fn list_events_in_range(
    store: &Store,
    start_iso: &str,
    end_iso: &str,
) -> Result<Vec<CalendarEventItem>, MobileError> {
    let start = parse_iso_date(start_iso)?;
    let end = parse_iso_date(end_iso)?;
    let end_exclusive = end.tomorrow().unwrap_or(end);
    let range = start..end_exclusive;
    let events = store.events_between(range.clone())?;

    let mut out = Vec::new();
    for event in &events {
        for occ in event.occurrences(range.clone(), 64) {
            out.push(CalendarEventItem {
                id: event.id().to_string(),
                title: event.title().to_owned(),
                notes: event.notes().to_owned(),
                location: event.location().to_owned(),
                date_iso: occ.start.date().to_string(),
                start_time: (!occ.all_day).then(|| format_time(occ.start.time())),
                end_time: (!occ.all_day).then(|| format_time(occ.end.time())),
                all_day: occ.all_day,
            });
        }
    }

    out.sort_by(|a, b| {
        a.date_iso
            .cmp(&b.date_iso)
            .then_with(|| b.all_day.cmp(&a.all_day))
            .then_with(|| a.start_time.cmp(&b.start_time))
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(out)
}

pub(crate) fn quick_add_event(
    store: &mut Store,
    input: &str,
    default_date_iso: Option<String>,
) -> Result<CalendarEventItem, MobileError> {
    let now = clock::now();
    let today = clock::today();
    let mut draft = parse_event(input, today);
    if let Some(default_iso) = default_date_iso
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let day = parse_iso_date(default_iso)?;
        draft = draft.on_default_day(day);
    }

    let mut event = if draft.all_day {
        Event::new_all_day(&draft.title, draft.start.date(), draft.last_day(), now)?
    } else {
        Event::new_timed(&draft.title, draft.start, draft.end, now)?
    };
    if let Some(rule) = draft.recurrence {
        event.set_recurrence(Some(rule), now)?;
    }
    if let Some(occasion) = draft.occasion {
        event.set_occasion(Some(occasion), now)?;
    }

    store.save_event(&event)?;
    enqueue_event(store, &event);
    Ok(event_to_dto(&event))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_event(
    store: &mut Store,
    id: &str,
    title: &str,
    notes: &str,
    location: &str,
    date_iso: &str,
    all_day: bool,
    start_time: Option<String>,
    end_time: Option<String>,
) -> Result<CalendarEventItem, MobileError> {
    let event_id = parse_event_id(id)?;
    let day = parse_iso_date(date_iso)?;
    let now = clock::now();
    let mut event = store
        .event(event_id)?
        .ok_or_else(|| MobileError::NotFound {
            reason: "Event not found".into(),
        })?;
    event.rename(title, now)?;
    event.set_notes(notes, now)?;
    event.set_location(location, now)?;
    if all_day {
        event.set_all_day_dates(day, day, now)?;
    } else {
        let start_t = start_time
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(parse_hhmm)
            .transpose()?
            .unwrap_or_else(|| Time::constant(9, 0, 0, 0));
        let end_t = end_time
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(parse_hhmm)
            .transpose()?
            .unwrap_or_else(|| {
                let next_h = (start_t.hour() + 1).min(23);
                Time::new(next_h, start_t.minute(), 0, 0).unwrap_or(Time::constant(23, 59, 0, 0))
            });
        let start_dt = day.to_datetime(start_t);
        let end_dt = if end_t > start_t {
            day.to_datetime(end_t)
        } else {
            day.tomorrow().unwrap_or(day).to_datetime(end_t)
        };
        event.set_times(start_dt, end_dt, now)?;
    }
    store.save_event(&event)?;
    enqueue_event(store, &event);
    Ok(event_to_dto(&event))
}

pub(crate) fn list_holidays_in_range(
    start_iso: &str,
    end_iso: &str,
    country_code: Option<String>,
) -> Result<Vec<HolidayItem>, MobileError> {
    let start = parse_iso_date(start_iso)?;
    let end = parse_iso_date(end_iso)?;
    let end_exclusive = end.tomorrow().unwrap_or(end);
    let detected = jiff::tz::TimeZone::system()
        .iana_name()
        .and_then(holidays::country_for_zone);
    let place = country_code
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(detected)
        .unwrap_or("US");
    let list = holidays::between(&[place], start..end_exclusive, false);
    Ok(list
        .into_iter()
        .map(|h| HolidayItem {
            date_iso: h.date.to_string(),
            short_name: holidays::short_name(h.name).to_owned(),
            name: h.name.to_owned(),
        })
        .collect())
}

pub(crate) fn list_trashed_events(store: &Store) -> Result<Vec<CalendarEventItem>, MobileError> {
    let events = store.trashed_events()?;
    Ok(events.iter().map(event_to_dto).collect())
}

pub(crate) fn delete_event_permanently(store: &mut Store, id: &str) -> Result<(), MobileError> {
    let event_id = parse_event_id(id)?;
    store.delete_event_permanently(event_id)?;
    if let Some(device_id) = sync::active_device_id(store) {
        let _ = enqueue_local_change(
            store,
            &device_id,
            Collection::Events,
            &event_id.to_string(),
            &["record"],
            true,
            &serde_json::json!({}),
            clock::now(),
        );
    }
    Ok(())
}

pub(crate) fn trash_event(store: &mut Store, id: &str) -> Result<(), MobileError> {
    let event_id = parse_event_id(id)?;
    let mut event = store
        .event(event_id)?
        .ok_or_else(|| MobileError::NotFound {
            reason: "Event not found".into(),
        })?;
    event.trash(clock::now())?;
    store.save_event(&event)?;
    enqueue_event(store, &event);
    Ok(())
}

pub(crate) fn restore_event(store: &mut Store, id: &str) -> Result<(), MobileError> {
    let event_id = parse_event_id(id)?;
    let mut event = store
        .event(event_id)?
        .ok_or_else(|| MobileError::NotFound {
            reason: "Event not found".into(),
        })?;
    event.restore(clock::now());
    store.save_event(&event)?;
    enqueue_event(store, &event);
    Ok(())
}
