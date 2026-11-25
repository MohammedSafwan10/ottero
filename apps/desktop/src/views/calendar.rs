//! Calendar screen: Month / Week / Day / Agenda over events and dated tasks.
//!
//! State: a mode and a selected day (`anchor`). The visible range follows
//! from both (see `calendar_layout`), and is reloaded from the Library on
//! every change and on `EventsChanged` / `TasksChanged`. Quick add parses
//! natural language; text without a day lands on the selected day.
//!
//! Keyboard (grid focused): arrows move the selected day, ⌘←/⌘→ the period,
//! T today, M/W/D/A switch views, Enter opens the day, N types a new event.
//! Grid rendering lives in `calendar_grid`.

use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Task as GpuiTask, Window, div, rems,
};
use jiff::Span;
use ottero_core::{Date, DateTime, EventId, TaskId, clock};
use ottero_ui::appearance;
use ottero_ui::components::{empty_state, page_header, segmented};

use crate::actions::{
    CALENDAR_CONTEXT, CalendarAgendaView, CalendarDayView, CalendarMonthView, CalendarNextDay,
    CalendarNextPeriod, CalendarNextWeek, CalendarOpenDay, CalendarPreviousDay,
    CalendarPreviousPeriod, CalendarPreviousWeek, CalendarQuickAdd, CalendarToday,
    CalendarWeekView,
};
use crate::library::{CalendarSpan, Library, LibraryEvent};
use crate::views::calendar_layout::{
    CalendarMode, first_weekday, format_span, format_time, step, title, visible_days,
};
use crate::views::event_detail::open_event_detail;
use crate::views::task_detail::open_task_detail;
use crate::views::task_row::notify_err;

/// How often the "now" line moves.
const CLOCK_TICK: Duration = Duration::from_secs(30);

/// What's being dragged: an event (or one occurrence of a repeating one) or a task.
#[derive(Clone)]
pub enum CalendarDrag {
    Event {
        id: EventId,
        /// Set for an occurrence of a repeating event: only it moves.
        series_date: Option<Date>,
        title: SharedString,
        all_day: bool,
        /// Length in minutes, kept when dropped on the time grid.
        minutes: i64,
        /// Pointer offset inside the dragged block, so a drop lands where the
        /// block's top edge is, not where the pointer is.
        grab_y: std::rc::Rc<std::cell::Cell<f32>>,
    },
    Task {
        id: TaskId,
        title: SharedString,
    },
}

impl CalendarDrag {
    fn title(&self) -> SharedString {
        match self {
            Self::Event { title, .. } | Self::Task { title, .. } => title.clone(),
        }
    }
}

impl Render for CalendarDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .shadow_md()
            .text_xs()
            .text_color(theme.popover_foreground)
            .child(self.title())
    }
}

pub struct CalendarView {
    pub(super) library: Entity<Library>,
    input: Entity<InputState>,
    hint: Option<SharedString>,
    pub(super) mode: CalendarMode,
    /// The selected day; the visible period contains it.
    pub(super) anchor: Date,
    pub(super) today: Date,
    pub(super) now: DateTime,
    pub(super) span: CalendarSpan,
    /// Holidays in the visible period (Settings → Calendar → Holidays).
    pub(super) holidays: crate::holidays::DayHolidays,
    /// Country of the time zone when last loaded (reload when you travel).
    here: Option<&'static str>,
    load_error: bool,
    pub(super) focus: FocusHandle,
    pub(super) time_scroll: ScrollHandle,
    /// The period the time grid was last scrolled for (scroll once per visit).
    pub(super) scrolled_for: Option<(CalendarMode, Date)>,
    /// Day, minute and length (minutes) of a drag over the time grid.
    pub(super) drop_target: Option<(Date, i32, i32)>,
    _clock: GpuiTask<()>,
    _subscriptions: Vec<Subscription>,
}

impl CalendarView {
    pub fn new(library: Entity<Library>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Add an event and press Enter"));
        let subscriptions = vec![
            cx.subscribe(&library, |this, _, event, cx| {
                if matches!(
                    event,
                    LibraryEvent::EventsChanged | LibraryEvent::TasksChanged
                ) {
                    this.reload(cx);
                }
            }),
            cx.subscribe_in(
                &input,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.add_from_input(window, cx),
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        this.hint = quick_add_hint(&text, this.anchor, cx);
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            // Week starts and the clock format live in the appearance settings.
            cx.observe_global::<gpui_kit::component::Theme>(|this, cx| this.reload(cx)),
        ];
        let clock_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CLOCK_TICK).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        });
        let today = clock::today();
        let mut this = Self {
            library,
            input,
            hint: None,
            mode: CalendarMode::Month,
            anchor: today,
            today,
            now: clock::now_local(),
            span: CalendarSpan::default(),
            holidays: crate::holidays::DayHolidays::default(),
            here: None,
            load_error: false,
            focus: cx.focus_handle(),
            time_scroll: ScrollHandle::new(),
            scrolled_for: None,
            drop_target: None,
            _clock: clock_task,
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    pub fn mode(&self) -> CalendarMode {
        self.mode
    }

    pub fn anchor(&self) -> Date {
        self.anchor
    }

    pub fn span(&self) -> &CalendarSpan {
        &self.span
    }

    pub fn holidays(&self) -> &crate::holidays::DayHolidays {
        &self.holidays
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let now = clock::now_local();
        if now.date() != self.today || crate::holidays::detected_country(cx) != self.here {
            self.reload(cx);
        } else if now.hour() != self.now.hour() || now.minute() != self.now.minute() {
            self.now = now;
            cx.notify();
        }
    }

    pub(super) fn first_weekday(&self, cx: &gpui_kit::App) -> jiff::civil::Weekday {
        first_weekday(appearance::settings(cx).week_start)
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let now = clock::now_local();
        // Follow "today" across midnight when it was selected.
        if self.anchor == self.today && now.date() != self.today {
            self.anchor = now.date();
        }
        self.today = now.date();
        self.now = now;
        let days = visible_days(self.mode, self.anchor, self.first_weekday(cx));
        self.here = crate::holidays::detected_country(cx);
        self.holidays = crate::holidays::DayHolidays::load(days.clone(), cx);
        match self.library.read(cx).calendar(days) {
            Ok(span) => {
                self.span = span;
                self.load_error = false;
            }
            Err(err) => {
                eprintln!("ottero: failed to load the calendar: {err:#}");
                self.span = CalendarSpan::default();
                self.load_error = true;
            }
        }
        cx.notify();
    }

    pub fn set_mode(&mut self, mode: CalendarMode, cx: &mut Context<Self>) {
        if self.mode != mode {
            self.mode = mode;
            self.reload(cx);
        }
    }

    /// Selects `day`; reloads only when it leaves the visible period.
    pub fn select(&mut self, day: Date, cx: &mut Context<Self>) {
        let before = visible_days(self.mode, self.anchor, self.first_weekday(cx));
        self.anchor = day;
        let after = visible_days(self.mode, self.anchor, self.first_weekday(cx));
        if before == after {
            cx.notify();
        } else {
            self.reload(cx);
        }
    }

    fn shift_days(&mut self, n: i64, cx: &mut Context<Self>) {
        if let Ok(day) = self.anchor.checked_add(Span::new().days(n)) {
            self.select(day, cx);
        }
    }

    fn shift_period(&mut self, dir: i64, cx: &mut Context<Self>) {
        self.select(step(self.mode, self.anchor, dir), cx);
    }

    pub fn go_today(&mut self, cx: &mut Context<Self>) {
        self.select(clock::today(), cx);
    }

    /// Opens a day in Day view (from a month cell or week header).
    pub(super) fn open_day(&mut self, day: Date, cx: &mut Context<Self>) {
        self.anchor = day;
        self.mode = CalendarMode::Day;
        self.reload(cx);
    }

    /// Focuses quick add (the "New event" command).
    pub fn focus_quick_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn add_from_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let day = self.anchor;
        let result = self
            .library
            .update(cx, |lib, cx| lib.quick_add_event(&text, day, cx));
        if let Ok(id) = &result {
            // Show where it landed: select its first day.
            if let Some(event) = self.library.read(cx).event(*id) {
                self.select(event.start().date(), cx);
            }
            self.input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.hint = None;
        }
        notify_err(result, window, cx);
    }

    /// Opens the event's details at the occurrence on `series_date`.
    pub(super) fn open_event(
        &self,
        id: EventId,
        series_date: Date,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        open_event_detail(
            id,
            Some(series_date),
            false,
            self.library.clone(),
            window,
            cx,
        );
    }

    pub(super) fn open_task(&self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        open_task_detail(id, self.library.clone(), window, cx);
    }

    /// Double-click on empty time: a one-hour "New event", title selected.
    pub(super) fn create_at(
        &mut self,
        start: DateTime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(end) = start.checked_add(Span::new().hours(1)) else {
            return;
        };
        let result = self.library.update(cx, |lib, cx| {
            lib.create_timed_event("New event", start, end, cx)
        });
        if let Some(id) = ottero_ui::feedback::report(result, window, cx) {
            open_event_detail(id, None, true, self.library.clone(), window, cx);
        }
    }

    /// Double-click on a month day: an all-day "New event", title selected.
    pub(super) fn create_all_day_on(
        &mut self,
        day: Date,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self
            .library
            .update(cx, |lib, cx| lib.create_all_day_event("New event", day, cx));
        if let Some(id) = ottero_ui::feedback::report(result, window, cx) {
            open_event_detail(id, None, true, self.library.clone(), window, cx);
        }
    }

    /// Dropped on a day (month cell or all-day row): events keep their
    /// times, timed events dropped on the all-day row become all-day when
    /// `make_all_day`; tasks get that due date.
    pub(super) fn drop_on_day(
        &mut self,
        day: Date,
        drag: &CalendarDrag,
        make_all_day: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let CalendarDrag::Event {
            id,
            series_date: Some(series_date),
            all_day,
            ..
        } = drag
        {
            if make_all_day && !all_day {
                ottero_ui::feedback::error_toast(
                    "One repeat can't become all-day on its own. Open the event to change them all.",
                    window,
                    cx,
                );
                return;
            }
            let (id, series_date) = (*id, *series_date);
            let result = self.library.update(cx, |lib, cx| {
                lib.move_occurrence_to_day(id, series_date, day, cx)
            });
            self.offer_occurrence_undo(id, series_date, result, window, cx);
            return;
        }
        let result = self.library.update(cx, |lib, cx| match drag {
            CalendarDrag::Event { id, all_day, .. } if make_all_day && !all_day => {
                lib.set_event_all_day(*id, day, day, cx)
            }
            CalendarDrag::Event { id, .. } => lib.move_event(*id, day, cx),
            CalendarDrag::Task { id, .. } => lib.set_task_due(*id, Some(day), cx),
        });
        notify_err(result, window, cx);
    }

    /// Dropped on the time grid at `drop_target`: events move there keeping
    /// their length (all-day ones become one hour); tasks take the day.
    pub(super) fn drop_on_grid(
        &mut self,
        drag: &CalendarDrag,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((day, minute, _)) = self.drop_target.take() else {
            return;
        };
        let start = day
            .to_datetime(jiff::civil::Time::midnight())
            .checked_add(Span::new().minutes(i64::from(minute)));
        let Ok(start) = start else { return };
        if let CalendarDrag::Event {
            id,
            series_date: Some(series_date),
            all_day,
            minutes,
            ..
        } = drag
        {
            if *all_day {
                ottero_ui::feedback::error_toast(
                    "One repeat can't get times on its own. Open the event to change them all.",
                    window,
                    cx,
                );
                return;
            }
            let Ok(end) = start.checked_add(Span::new().minutes(*minutes)) else {
                return;
            };
            let (id, series_date) = (*id, *series_date);
            let result = self.library.update(cx, |lib, cx| {
                lib.move_occurrence(id, series_date, start, end, cx)
            });
            self.offer_occurrence_undo(id, series_date, result, window, cx);
            return;
        }
        let result = self.library.update(cx, |lib, cx| match drag {
            CalendarDrag::Event {
                id,
                all_day,
                minutes,
                ..
            } => {
                let length = if *all_day { 60 } else { *minutes };
                match start.checked_add(Span::new().minutes(length)) {
                    Ok(end) => lib.set_event_times(*id, start, end, cx),
                    Err(_) => Ok(()),
                }
            }
            CalendarDrag::Task { id, .. } => lib.set_task_due(*id, Some(day), cx),
        });
        notify_err(result, window, cx);
    }

    /// After moving one occurrence by drag: say so, and offer Undo.
    fn offer_occurrence_undo(
        &self,
        id: EventId,
        series_date: Date,
        result: anyhow::Result<Option<ottero_core::Exception>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(previous) = ottero_ui::feedback::report(result, window, cx) else {
            return;
        };
        let library = self.library.clone();
        ottero_ui::feedback::undo_toast("Moved this event only.", window, cx, move |window, cx| {
            notify_err(
                library.update(cx, |lib, cx| {
                    lib.restore_occurrence(id, series_date, previous, cx)
                }),
                window,
                cx,
            );
        });
    }

    /// Whether a routine (repeats 3+ times a week) is behind `occ`.
    pub(super) fn is_routine(&self, occ: &ottero_core::Occurrence) -> bool {
        self.span
            .events
            .get(&occ.event)
            .is_some_and(ottero_core::Event::is_routine)
    }

    /// Whether `occ` is shown (routines can be hidden from View options).
    pub(super) fn shows(&self, occ: &ottero_core::Occurrence, cx: &gpui_kit::App) -> bool {
        let prefs = appearance::settings(cx);
        let occasion = self
            .span
            .events
            .get(&occ.event)
            .is_some_and(|e| e.occasion().is_some());
        (prefs.calendar_routines || !self.is_routine(occ))
            && (prefs.calendar_occasions || !occasion)
    }

    #[doc(hidden)]
    pub fn shows_for_test(&self, occ: &ottero_core::Occurrence, cx: &gpui_kit::App) -> bool {
        self.shows(occ, cx)
    }

    /// Quiet lines naming `day`'s holidays (agenda and day panel).
    pub(super) fn holiday_rows(&self, day: Date, cx: &gpui_kit::App) -> Vec<gpui_kit::AnyElement> {
        let theme = cx.theme();
        self.holidays
            .on(day)
            .iter()
            .map(|h| {
                let public = h.kind == ottero_core::holidays::HolidayKind::Holiday;
                let note = crate::holidays::describe(h);
                h_flex()
                    .gap_3()
                    .px_2()
                    .py_1()
                    .items_start()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(
                        h_flex()
                            .w(rems(3.75))
                            .flex_shrink_0()
                            .pt(rems(0.1875))
                            .child(
                                Icon::new(if public {
                                    Lucide::Flag
                                } else {
                                    Lucide::Sparkles
                                })
                                .xsmall()
                                .text_color(theme.muted_foreground),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            // Full names wrap (some are long: "Pratihar
                            // Shashthi or Surya Shashthi (Chhath Puja)").
                            .child(
                                div()
                                    .whitespace_normal()
                                    .when(public, |s| s.text_color(theme.foreground))
                                    .child(self.holidays.label(h)),
                            )
                            .child(
                                div().text_xs().whitespace_normal().child(
                                    note.split_once(" · ")
                                        .map_or(note.clone(), |(_, rest)| rest.to_owned()),
                                ),
                            ),
                    )
                    .into_any_element()
            })
            .collect()
    }

    pub(super) fn toggle_task(&self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        notify_err(
            self.library.update(cx, |lib, cx| lib.toggle_task(id, cx)),
            window,
            cx,
        );
    }
}

/// Picks an `.ics` file (Google, Apple or Outlook export) and imports it.
pub fn import_ics_file(
    library: gpui_kit::Entity<crate::library::Library>,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) {
    let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("Import".into()),
    });
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let text = cx
                .background_executor()
                .spawn(async move { std::fs::read(path) })
                .await;
            _ = cx.update(|window, cx| {
                let Ok(bytes) = text else {
                    ottero_ui::feedback::error_toast("Couldn't read that file.", window, cx);
                    return;
                };
                let text = String::from_utf8_lossy(&bytes);
                let result = library.update(cx, |lib, cx| lib.import_ics(&text, cx));
                if let Some(report) = ottero_ui::feedback::report(result, window, cx) {
                    gpui_kit::component::WindowExt::push_notification(window, report.message(), cx);
                }
            });
        })
        .detach();
}

/// The "View options" menu: what the calendar shows (routines, holidays),
/// then import and deleted events.
fn view_options(cx: &gpui_kit::App) -> impl IntoElement {
    let prefs = appearance::settings(cx);
    let routines = prefs.calendar_routines;
    let occasions = prefs.calendar_occasions;
    let holidays = prefs.holidays;
    let observances = prefs.holiday_observances;
    let where_you_are = prefs.holidays_where_you_are;
    let countries: Vec<&'static str> = crate::holidays::chosen(cx)
        .into_iter()
        .filter_map(ottero_core::holidays::country_name)
        .collect();
    let countries_label: SharedString = if countries.is_empty() {
        "No countries chosen".into()
    } else {
        format!("Holidays: {}", countries.join(", ")).into()
    };
    Button::new("calendar-view-options")
        .ghost()
        .small()
        .icon(Lucide::SlidersHorizontal)
        .tooltip("View options")
        .dropdown_menu(move |menu, _, _| {
            menu.item(
                PopupMenuItem::new("Show routines")
                    .checked(routines)
                    .on_click(move |_, _, cx| {
                        appearance::update(cx, |s| s.calendar_routines = !routines)
                    }),
            )
            .label("Events repeating 3+ times a week")
            .item(
                PopupMenuItem::new("Birthdays and anniversaries")
                    .checked(occasions)
                    .on_click(move |_, _, cx| {
                        appearance::update(cx, |s| s.calendar_occasions = !occasions)
                    }),
            )
            .separator()
            .item(
                PopupMenuItem::new("Show holidays")
                    .checked(holidays)
                    .on_click(move |_, _, cx| appearance::update(cx, |s| s.holidays = !holidays)),
            )
            .item(
                PopupMenuItem::new("Festivals and observances")
                    .checked(observances)
                    .disabled(!holidays)
                    .on_click(move |_, _, cx| {
                        appearance::update(cx, |s| s.holiday_observances = !observances)
                    }),
            )
            .item(
                PopupMenuItem::new("Holidays where you are")
                    .checked(where_you_are)
                    .disabled(!holidays)
                    .on_click(move |_, _, cx| {
                        appearance::update(cx, |s| s.holidays_where_you_are = !where_you_are)
                    }),
            )
            .label(countries_label.clone())
            .item(
                PopupMenuItem::new("Choose countries…")
                    .icon(Lucide::Flag)
                    .action(Box::new(crate::actions::GoSettings)),
            )
            .separator()
            .item(
                PopupMenuItem::new("Import calendar file…")
                    .icon(Lucide::CalendarPlus)
                    .action(Box::new(crate::actions::ImportCalendar)),
            )
            .item(
                PopupMenuItem::new("Deleted events…")
                    .icon(Lucide::Trash)
                    .action(Box::new(crate::actions::ShowDeletedEvents)),
            )
        })
}

/// "Lunch with Sam — Fri, Nov 13 · 1 – 2 PM · Every week".
fn quick_add_hint(text: &str, day: Date, cx: &gpui_kit::App) -> Option<SharedString> {
    if text.trim().is_empty() {
        return None;
    }
    let today = clock::today();
    let draft = ottero_core::parse_event(text, today).on_default_day(day);
    let clock = appearance::settings(cx).clock;
    let first = draft.start.date();
    let when = crate::views::relative_day(first, today);
    let mut parts = Vec::new();
    if draft.all_day {
        let last = draft.last_day();
        if last == first {
            parts.push(format!("{when} · All day"));
        } else {
            parts.push(format!(
                "{} – {}",
                when,
                crate::views::relative_day(last, today)
            ));
        }
    } else {
        parts.push(format!(
            "{when} · {}",
            format_span(draft.start, draft.end, clock)
        ));
    }
    if let Some(rule) = draft.recurrence {
        parts.push(rule.to_string());
    }
    if let Some(o) = draft.occasion {
        parts.push(match o.kind {
            ottero_core::OccasionKind::Birthday => "Birthday".to_owned(),
            ottero_core::OccasionKind::Anniversary => "Anniversary".to_owned(),
        });
        parts.extend(o.note(first));
    }
    Some(format!("{} — {}", draft.title, parts.join(" · ")).into())
}

impl Focusable for CalendarView {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl CalendarView {
    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let first = self.first_weekday(cx);
        let heading = title(self.mode, self.anchor, first, self.today);
        let view = cx.entity().downgrade();
        let modes = segmented(
            "calendar-modes",
            &CalendarMode::OPTIONS,
            self.mode,
            move |mode, _, cx| {
                _ = view.update(cx, |this, cx| this.set_mode(mode, cx));
            },
        );
        let nav = h_flex()
            .gap_1()
            .items_center()
            .child(
                Button::new("calendar-previous")
                    .ghost()
                    .small()
                    .icon(Lucide::ChevronLeft)
                    .tooltip_with_action("Previous", &CalendarPreviousPeriod, None)
                    .on_click(cx.listener(|this, _, _, cx| this.shift_period(-1, cx))),
            )
            .child(
                Button::new("calendar-today")
                    .outline()
                    .small()
                    .label("Today")
                    .tooltip_with_action("Go to today", &CalendarToday, None)
                    .on_click(cx.listener(|this, _, _, cx| this.go_today(cx))),
            )
            .child(
                Button::new("calendar-next")
                    .ghost()
                    .small()
                    .icon(Lucide::ChevronRight)
                    .tooltip_with_action("Next", &CalendarNextPeriod, None)
                    .on_click(cx.listener(|this, _, _, cx| this.shift_period(1, cx))),
            );
        let theme = cx.theme();
        v_flex()
            .w_full()
            .px_6()
            .pt_6()
            .pb_3()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(div().min_w_0().child(page_header(heading, None, cx)))
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .items_center()
                            .child(nav)
                            .child(modes)
                            .child(view_options(cx)),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(Input::new(&self.input).id("event-input").prefix(
                        Icon::new(Lucide::Plus)
                            .small()
                            .text_color(theme.muted_foreground),
                    ))
                    .child(
                        div()
                            .h(rems(1.25))
                            .px_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(self.hint.clone().unwrap_or_else(|| {
                                "Try “Lunch with Sam fri 1pm”, “Trip nov 20-22”, “Standup 9:30-9:45 every weekday”".into()
                            })),
                    ),
            )
    }

    fn agenda(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let clock = appearance::settings(cx).clock;
        let mut groups = Vec::new();
        let mut day = self.span.days.start;
        while day < self.span.days.end {
            let events: Vec<_> = self
                .span
                .on_day(day)
                .filter(|o| self.shows(o, cx))
                .copied()
                .collect();
            let tasks: Vec<_> = self.span.tasks_on(day).cloned().collect();
            if !events.is_empty() || !tasks.is_empty() || !self.holidays.on(day).is_empty() {
                groups.push((day, events, tasks));
            }
            day = day.tomorrow().unwrap_or(self.span.days.end);
        }
        let today = self.today;
        let content = if groups.is_empty() {
            let (title, desc) = if self.load_error {
                (
                    "Couldn't load the calendar",
                    "Your events are still on disk. Restart Ottero; if this keeps happening, check the data folder in Settings.",
                )
            } else {
                (
                    "Nothing in the next 30 days",
                    "Add an event above, like “Dentist fri 3pm”, or give a task a due date.",
                )
            };
            empty_state(Lucide::CalendarDays, title, desc, cx).into_any_element()
        } else {
            v_flex()
                .gap_5()
                .children(groups.into_iter().map(|(day, events, tasks)| {
                    let label = match (day - today).get_days() {
                        0 => format!("Today · {}", day.strftime("%a, %b %-d")),
                        1 => format!("Tomorrow · {}", day.strftime("%a, %b %-d")),
                        _ => day.strftime("%A, %b %-d").to_string(),
                    };
                    let rows = events.into_iter().filter_map(|occ| {
                        let event = self.span.events.get(&occ.event)?;
                        let id = event.id();
                        let series = occ.series_date;
                        let when: SharedString = if occ.all_day {
                            "All day".into()
                        } else if occ.start.date() < day {
                            format!("Until {}", format_time(occ.end.time(), clock)).into()
                        } else {
                            format_span(occ.start, occ.end, clock).into()
                        };
                        Some(
                            h_flex()
                                .id(SharedString::from(format!("agenda-{id}-{day}")))
                                .gap_3()
                                .px_2()
                                .py_1p5()
                                .rounded(theme.radius)
                                .hover(|s| s.bg(theme.muted))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_event(id, series, window, cx)
                                }))
                                .child(
                                    div()
                                        .w(rems(8.))
                                        .flex_shrink_0()
                                        .text_sm()
                                        .text_color(theme.muted_foreground)
                                        .child(when),
                                )
                                .child(
                                    h_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_2()
                                        .children(self.occasion_icon(&occ).map(|i| {
                                            Icon::new(i).xsmall().text_color(theme.muted_foreground)
                                        }))
                                        .child(
                                            div().truncate().text_sm().child(self.title_of(&occ)),
                                        )
                                        .when(!event.location().is_empty(), |this| {
                                            this.child(
                                                div()
                                                    .truncate()
                                                    .text_sm()
                                                    .text_color(theme.muted_foreground)
                                                    .child(event.location().to_owned()),
                                            )
                                        }),
                                )
                                .when(event.recurrence().is_some(), |this| {
                                    this.child(
                                        Icon::new(Lucide::Repeat)
                                            .xsmall()
                                            .text_color(theme.muted_foreground),
                                    )
                                })
                                .into_any_element(),
                        )
                    });
                    let task_rows = tasks.into_iter().map(|task| {
                        let id = task.id();
                        let done = !task.status().is_open();
                        h_flex()
                            .id(SharedString::from(format!("agenda-task-{id}")))
                            .gap_3()
                            .px_2()
                            .py_1p5()
                            .rounded(theme.radius)
                            .hover(|s| s.bg(theme.muted))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_task(id, window, cx)
                            }))
                            .child(
                                h_flex().w(rems(8.)).flex_shrink_0().child(
                                    Button::new(SharedString::from(format!("agenda-check-{id}")))
                                        .ghost()
                                        .xsmall()
                                        .icon(if done {
                                            Lucide::CircleCheckBig
                                        } else {
                                            Lucide::Circle
                                        })
                                        .tooltip(if done { "Mark as not done" } else { "Complete" })
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.toggle_task(id, window, cx);
                                        })),
                                ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .when(done, |this| {
                                        this.line_through().text_color(theme.muted_foreground)
                                    })
                                    .child(task.title().to_owned()),
                            )
                            .into_any_element()
                    });
                    v_flex()
                        .w_full()
                        .p_4()
                        .gap_1()
                        .rounded(theme.radius_lg)
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.group_box)
                        .when(theme.shadow, |s| s.shadow_xs())
                        .child(
                            div()
                                .px_2()
                                .pb_1()
                                .text_sm()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .when(day == today, |this| this.text_color(theme.primary))
                                .child(label),
                        )
                        .children(self.holiday_rows(day, cx))
                        .children(rows)
                        .children(task_rows)
                }))
                .into_any_element()
        };
        div()
            .id("calendar-agenda")
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .w_full()
                    .max_w(rems(48.))
                    .mx_auto()
                    .px_6()
                    .pb_6()
                    .child(content),
            )
    }
}

impl Render for CalendarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.mode {
            CalendarMode::Month => self.month(window, cx).into_any_element(),
            CalendarMode::Week | CalendarMode::Day => self.time_grid(window, cx).into_any_element(),
            CalendarMode::Agenda => self.agenda(cx).into_any_element(),
        };
        v_flex().size_full().child(self.header(cx)).child(
            div()
                .id("calendar-body")
                .track_focus(&self.focus)
                .key_context(CALENDAR_CONTEXT)
                .flex_1()
                .min_h_0()
                .on_action(cx.listener(|this, _: &CalendarToday, _, cx| this.go_today(cx)))
                .on_action(
                    cx.listener(|this, _: &CalendarPreviousPeriod, _, cx| {
                        this.shift_period(-1, cx)
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &CalendarNextPeriod, _, cx| this.shift_period(1, cx)),
                )
                .on_action(
                    cx.listener(|this, _: &CalendarPreviousDay, _, cx| this.shift_days(-1, cx)),
                )
                .on_action(cx.listener(|this, _: &CalendarNextDay, _, cx| this.shift_days(1, cx)))
                .on_action(
                    cx.listener(|this, _: &CalendarPreviousWeek, _, cx| this.shift_days(-7, cx)),
                )
                .on_action(cx.listener(|this, _: &CalendarNextWeek, _, cx| this.shift_days(7, cx)))
                .on_action(cx.listener(|this, _: &CalendarOpenDay, _, cx| {
                    let day = this.anchor;
                    this.open_day(day, cx)
                }))
                .on_action(cx.listener(|this, _: &CalendarMonthView, _, cx| {
                    this.set_mode(CalendarMode::Month, cx)
                }))
                .on_action(cx.listener(|this, _: &CalendarWeekView, _, cx| {
                    this.set_mode(CalendarMode::Week, cx)
                }))
                .on_action(cx.listener(|this, _: &CalendarDayView, _, cx| {
                    this.set_mode(CalendarMode::Day, cx)
                }))
                .on_action(cx.listener(|this, _: &CalendarAgendaView, _, cx| {
                    this.set_mode(CalendarMode::Agenda, cx)
                }))
                .on_action(cx.listener(|this, _: &CalendarQuickAdd, window, cx| {
                    this.focus_quick_add(window, cx)
                }))
                .child(body),
        )
    }
}
