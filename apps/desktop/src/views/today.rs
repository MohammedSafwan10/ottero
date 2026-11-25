//! Today: one calm page for the day.
//!
//! Header (date, greeting, Today's note, Customize), then the sections from
//! `today_layout` in the user's order: at a glance, schedule, habits, due
//! tasks, this week, next 7 days, recent notes. Half-width sections pair up
//! on wide windows and stack when narrow. Each section is a card with a title
//! row, an optional action, content, and its own empty state.
//!
//! State: everything is read from the Library in `reload` (on any
//! `LibraryEvent`, on navigation and when the window comes back), plus a
//! 30-second tick so the "now" marker and "in 20 min" stay true.

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::chart::BarChart;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Corners, Div, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task as GpuiTask,
    TestSupportExt as _, Window, div, px, relative, rems,
};
use ottero_core::habit_insights::{HabitHistory, day_rate};
use ottero_core::{Date, DateTime, Occurrence, Task, TaskId, clock};
use ottero_storage::{NoteSummary, Progress, TaskCounts, TaskFilter};
use ottero_ui::appearance;

use crate::library::{CalendarSpan, HabitSummary, Library};
use crate::views::calendar_layout::{format_span, format_time};
use crate::views::event_detail::open_event_detail;
use crate::views::habit_widgets::{add_days, caption};
use crate::views::task_detail::open_task_detail;
use crate::views::task_row::task_row;
use crate::views::today_layout::{self, TodaySection};
use crate::views::{Navigate, Route, relative_day};

const RECENT_NOTES: usize = 5;
const UPCOMING_DAYS: i64 = 7;
const UPCOMING_MAX: usize = 8;
const TASKS_MAX: usize = 6;
const CLOCK_TICK: Duration = Duration::from_secs(30);

/// Asks the shell to open today's daily note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenDailyNoteRequest;

type ClickFn = Box<dyn Fn(&gpui_kit::ClickEvent, &mut Window, &mut App)>;

/// One line in "Next 7 days".
#[derive(Debug, Clone)]
enum Upcoming {
    Event(Occurrence),
    Task(Task),
}

pub struct TodayView {
    library: Entity<Library>,
    today: Date,
    now: DateTime,
    counts: TaskCounts,
    due: Vec<Task>,
    week: Vec<(Date, usize)>,
    /// Habit completion per day for the week chart (`None`: nothing expected).
    habit_week: Vec<Option<f64>>,
    progress: HashMap<TaskId, Progress>,
    recent: Vec<NoteSummary>,
    habits: Vec<HabitSummary>,
    /// Today through the next 7 days.
    span: CalendarSpan,
    /// Holidays over the same days.
    holidays: crate::holidays::DayHolidays,
    has_daily_note: bool,
    focus: FocusHandle,
    _clock: GpuiTask<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Navigate> for TodayView {}
impl EventEmitter<OpenDailyNoteRequest> for TodayView {}

impl TodayView {
    pub fn new(library: Entity<Library>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.subscribe(&library, |this, _, _, cx| this.reload(cx)),
            // Section order, visibility and holidays live in the appearance settings.
            cx.observe_global::<gpui_kit::component::Theme>(|this, cx| this.reload(cx)),
        ];
        let clock_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CLOCK_TICK).await;
                let alive = this.update(cx, |this, cx| {
                    if clock::today() == this.today {
                        this.now = clock::now_local();
                        cx.notify();
                    } else {
                        this.reload(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let mut this = Self {
            library,
            today: clock::today(),
            now: clock::now_local(),
            counts: TaskCounts::default(),
            due: Vec::new(),
            week: Vec::new(),
            habit_week: Vec::new(),
            progress: HashMap::new(),
            recent: Vec::new(),
            habits: Vec::new(),
            span: CalendarSpan::default(),
            holidays: crate::holidays::DayHolidays::default(),
            has_daily_note: false,
            focus: cx.focus_handle(),
            _clock: clock_task,
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    /// Refreshes everything, including the date (the app may stay open past midnight).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.today = clock::today();
        self.now = clock::now_local();
        self.holidays = crate::holidays::DayHolidays::load(
            self.today..add_days(self.today, UPCOMING_DAYS + 1),
            cx,
        );
        let lib = self.library.read(cx);
        self.counts = lib.task_counts().unwrap_or_default();
        self.due = lib.tasks(TaskFilter::Today(self.today)).unwrap_or_default();
        self.week = lib.completions_by_day(7);
        self.progress = lib.subtask_progress();
        self.has_daily_note = lib.daily_note_exists(self.today);
        self.recent = lib
            .notes(false)
            .map(|mut notes| {
                notes.truncate(RECENT_NOTES);
                notes
            })
            .unwrap_or_default();
        self.habits = lib.habits(false).unwrap_or_default();
        self.span = lib
            .calendar(self.today..add_days(self.today, UPCOMING_DAYS + 1))
            .unwrap_or_default();
        let hist: Vec<HabitHistory<'_>> = self
            .habits
            .iter()
            .map(|h| HabitHistory {
                habit: &h.habit,
                counts: &h.counts,
                first: h.first,
            })
            .collect();
        self.habit_week = self
            .week
            .iter()
            .map(|(day, _)| day_rate(&hist, *day, self.today))
            .collect();
        cx.notify();
    }

    fn greeting(&self) -> &'static str {
        match self.now.hour() {
            5..=11 => "Good morning",
            12..=16 => "Good afternoon",
            _ => "Good evening",
        }
    }

    fn today_events(&self) -> Vec<Occurrence> {
        self.span.on_day(self.today).copied().collect()
    }

    /// The title; a birthday or anniversary adds its years ("Mom's
    /// birthday · turns 60").
    fn title_of(&self, occ: &Occurrence) -> String {
        let Some(e) = self.span.events.get(&occ.event) else {
            return String::new();
        };
        match e.occasion().and_then(|o| o.note(occ.start.date())) {
            Some(note) => format!("{} · {note}", e.title()),
            None => e.title().to_owned(),
        }
    }

    fn occasion_icon(&self, occ: &Occurrence) -> Option<Lucide> {
        let o = self.span.events.get(&occ.event)?.occasion()?;
        Some(match o.kind {
            ottero_core::OccasionKind::Birthday => Lucide::Cake,
            ottero_core::OccasionKind::Anniversary => Lucide::Gift,
        })
    }

    fn is_routine(&self, occ: &Occurrence) -> bool {
        self.span
            .events
            .get(&occ.event)
            .is_some_and(ottero_core::Event::is_routine)
    }

    /// "in 25 min", "in 2 h 10 min", "now".
    fn until(&self, start: DateTime) -> String {
        let minutes = (start - self.now)
            .total(jiff::Unit::Minute)
            .map_or(0, |m| m.round() as i64);
        match minutes {
            m if m <= 0 => "now".into(),
            m if m < 60 => format!("in {m} min"),
            m if m % 60 == 0 => format!("in {} h", m / 60),
            m => format!("in {} h {} min", m / 60, m % 60),
        }
    }

    // ---- Building blocks -------------------------------------------------

    fn card(id: &'static str, cx: &App) -> gpui_kit::Stateful<Div> {
        let theme = cx.theme();
        v_flex()
            .id(id)
            .gap_3()
            .p_5()
            .bg(theme.group_box)
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .when(theme.shadow, |s| s.shadow_xs())
    }

    fn title_row(
        &self,
        title: &'static str,
        meta: Option<String>,
        action: Option<(&'static str, &'static str, Navigate)>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .h(rems(1.5))
            .justify_between()
            .items_center()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .children(
                        meta.map(|m| div().text_xs().text_color(theme.muted_foreground).child(m)),
                    ),
            )
            .children(action.map(|(id, label, nav)| {
                Button::new(id)
                    .ghost()
                    .xsmall()
                    .label(label)
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(nav)))
            }))
    }

    fn quiet(text: impl Into<SharedString>, cx: &App) -> Div {
        div()
            .py_2()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text.into())
    }

    fn meter(value: f32, cx: &App) -> Div {
        let theme = cx.theme();
        div()
            .h(rems(0.25))
            .w_full()
            .rounded_full()
            .bg(theme.muted)
            .child(
                div()
                    .h_full()
                    .w(relative(value.clamp(0., 1.)))
                    .rounded_full()
                    .bg(theme.primary),
            )
    }

    // ---- Sections --------------------------------------------------------

    fn glance(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let clock = appearance::settings(cx).clock;
        let events = self.today_events();
        let next = events
            .iter()
            .filter(|o| !o.all_day && o.end > self.now)
            .find(|o| !self.is_routine(o))
            .or_else(|| events.iter().find(|o| !o.all_day && o.end > self.now));
        let (next_value, next_note): (String, String) = match next {
            Some(o) if o.start <= self.now => (
                self.title_of(o),
                format!("Now · until {}", format_time(o.end.time(), clock)),
            ),
            Some(o) => (
                self.title_of(o),
                format!(
                    "{} · {}",
                    format_time(o.start.time(), clock),
                    self.until(o.start)
                ),
            ),
            None if events.is_empty() => ("Free day".into(), "Nothing on the calendar".into()),
            None => ("All done".into(), format!("{} earlier today", events.len())),
        };
        let left = self.counts.due_today + self.counts.overdue;
        let (task_value, task_note) = if left == 0 {
            (
                "All clear".to_owned(),
                format!("{} done today", self.counts.completed_today),
            )
        } else {
            let mut note = format!("{} done today", self.counts.completed_today);
            if self.counts.overdue > 0 {
                note = format!("{} overdue · {note}", self.counts.overdue);
            }
            (format!("{left} to do"), note)
        };
        let due: Vec<&HabitSummary> = self.habits.iter().filter(|h| h.due_today).collect();
        let done = due.iter().filter(|h| h.done_today()).count();
        let share = if due.is_empty() {
            0.
        } else {
            done as f32 / due.len() as f32
        };
        let (habit_value, habit_note) = if self.habits.is_empty() {
            ("No habits".to_owned(), "Add one in Habits".to_owned())
        } else if due.is_empty() {
            ("Rest day".to_owned(), "Nothing scheduled".to_owned())
        } else {
            (
                format!("{done} of {}", due.len()),
                if done == due.len() {
                    "All kept today".to_owned()
                } else {
                    format!("{} left", due.len() - done)
                },
            )
        };
        let overdue = self.counts.overdue > 0;
        let column = |id: &'static str,
                      icon: Lucide,
                      label: &'static str,
                      value: String,
                      note: String,
                      extra: Option<Div>,
                      route: Route,
                      first: bool,
                      cx: &mut Context<Self>| {
            v_flex()
                .id(id)
                .flex_1()
                .min_w(rems(10.))
                .gap_1()
                .px_4()
                .py_3()
                .rounded(theme.radius)
                .when(!first, |s| s.border_l_1().border_color(theme.border))
                .hover(|s| s.bg(theme.list_hover))
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(Navigate::To(route))))
                .test_support()
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(Icon::new(icon).xsmall())
                        .child(label),
                )
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(value),
                )
                .child(
                    div()
                        .text_xs()
                        .truncate()
                        .text_color(theme.muted_foreground)
                        .child(note),
                )
                .children(extra)
        };
        h_flex()
            .id("glance-card")
            .flex_wrap()
            // Equal-height columns, so labels share one baseline.
            .items_stretch()
            .py_1()
            .bg(theme.group_box)
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .when(theme.shadow, |s| s.shadow_xs())
            .child(column(
                "glance-next",
                Lucide::CalendarClock,
                "Next up",
                next_value,
                next_note,
                None,
                Route::Calendar,
                true,
                cx,
            ))
            .child(column(
                "glance-tasks",
                if overdue {
                    Lucide::TriangleAlert
                } else {
                    Lucide::CircleCheckBig
                },
                "Tasks",
                task_value,
                task_note,
                None,
                Route::Tasks,
                false,
                cx,
            ))
            .child(column(
                "glance-habits",
                Lucide::Sprout,
                "Habits",
                habit_value,
                habit_note,
                (!due.is_empty()).then(|| div().pt_1().child(Self::meter(share, cx))),
                Route::Habits,
                false,
                cx,
            ))
            .into_any_element()
    }

    fn agenda(&self, cx: &mut Context<Self>) -> AnyElement {
        let clock = appearance::settings(cx).clock;
        let events = self.today_events();
        let count = events.len();
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut marked = false;
        for occ in &events {
            // The now marker goes before the first event that hasn't started.
            if !marked && !occ.all_day && occ.start > self.now {
                marked = true;
                rows.push(self.now_marker(clock, cx));
            }
            rows.push(self.agenda_row(occ, clock, cx));
        }
        if !marked && !events.is_empty() {
            rows.push(self.now_marker(clock, cx));
        }
        Self::card("agenda-card", cx)
            .child(self.title_row(
                "Today's schedule",
                (count > 0).then(|| format!("{count}")),
                Some((
                    "today-open-calendar",
                    "Calendar",
                    Navigate::To(Route::Calendar),
                )),
                cx,
            ))
            .map(|c| {
                if events.is_empty() {
                    c.child(Self::quiet(
                        "Nothing scheduled. Add an event in Calendar, like “Lunch 1pm”.",
                        cx,
                    ))
                } else {
                    c.child(v_flex().gap_0p5().children(rows))
                }
            })
            .into_any_element()
    }

    fn now_marker(&self, clock: ottero_ui::appearance::ClockFormat, cx: &App) -> AnyElement {
        let theme = cx.theme();
        h_flex()
            .gap_2()
            .items_center()
            .py_0p5()
            .child(
                div()
                    .w(rems(4.5))
                    .flex_shrink_0()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.primary)
                    .child(format_time(self.now.time(), clock)),
            )
            .child(div().size_1p5().rounded_full().bg(theme.primary))
            .child(div().flex_1().h(px(1.)).bg(theme.primary.opacity(0.6)))
            .into_any_element()
    }

    fn agenda_row(
        &self,
        occ: &Occurrence,
        clock: ottero_ui::appearance::ClockFormat,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let id = occ.event;
        let series = occ.series_date;
        let event = self.span.events.get(&id);
        let location = event.map(|e| e.location().to_owned()).unwrap_or_default();
        let past = occ.end <= self.now && !occ.all_day;
        let current = !occ.all_day && occ.start <= self.now && occ.end > self.now;
        let routine = self.is_routine(occ);
        let when: SharedString = if occ.all_day {
            "All day".into()
        } else if occ.start.date() < self.today {
            format!("until {}", format_time(occ.end.time(), clock)).into()
        } else {
            format_time(occ.start.time(), clock).into()
        };
        let detail = if occ.all_day {
            location
        } else {
            let span = format_span(occ.start, occ.end, clock);
            if location.is_empty() {
                span
            } else {
                format!("{span} · {location}")
            }
        };
        let bar_color = if routine {
            theme.muted_foreground.opacity(0.4)
        } else if current {
            theme.warning
        } else {
            theme.info
        };
        let library = self.library.clone();
        h_flex()
            .id(ElementId::Name(format!("today-event-{id}").into()))
            .gap_2p5()
            .items_stretch()
            .px_2p5()
            .py_2()
            .rounded(theme.radius)
            .when(current, |s| s.bg(theme.warning.opacity(0.14)))
            .when(!current && !routine, |s| s.bg(theme.muted.opacity(0.45)))
            .hover(|s| s.bg(theme.list_hover))
            .when(past, |s| s.opacity(0.5))
            .on_click(move |_, window, cx| {
                open_event_detail(id, Some(series), false, library.clone(), window, cx)
            })
            .test_support()
            .child(
                div()
                    .w(rems(4.25))
                    .flex_shrink_0()
                    .pt(rems(0.0625))
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(when),
            )
            .child(
                div()
                    .w(px(3.))
                    .my(px(1.))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(bar_color),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .when(!routine, |s| s.font_weight(FontWeight::MEDIUM))
                                    .when(routine, |s| s.text_color(theme.muted_foreground))
                                    .child(self.title_of(occ)),
                            )
                            .when(current, |s| {
                                s.child(
                                    div()
                                        .flex_shrink_0()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme.primary)
                                        .child("Now"),
                                )
                            }),
                    )
                    .when(!detail.is_empty(), |s| {
                        s.child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(detail),
                        )
                    }),
            )
            .into_any_element()
    }

    fn habits_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let due: Vec<&HabitSummary> = self.habits.iter().filter(|h| h.due_today).collect();
        let done = due.iter().filter(|h| h.done_today()).count();
        let meta = (!due.is_empty()).then(|| format!("{done} of {}", due.len()));
        Self::card("habits-card", cx)
            .child(self.title_row(
                "Habits",
                meta,
                Some((
                    "today-open-habits",
                    "All habits",
                    Navigate::To(Route::Habits),
                )),
                cx,
            ))
            .map(|c| {
                if self.habits.is_empty() {
                    c.child(Self::quiet(
                        "No habits yet. Add one in Habits, like “Read 20 pages daily”.",
                        cx,
                    ))
                } else if due.is_empty() {
                    c.child(Self::quiet("Nothing scheduled today. Enjoy the rest.", cx))
                } else {
                    c.child(h_flex().flex_wrap().gap_3().children(due.iter().map(|h| {
                        let id = h.habit.id();
                        let done = h.done_today();
                        let count = h.count_on(self.today);
                        let target = h.habit.target();
                        let streak = h.stats.current_streak;
                        let library = self.library.clone();
                        let today = self.today;
                        v_flex()
                            .w(rems(4.5))
                            .gap_1p5()
                            .items_center()
                            .child(
                                div()
                                    .id(ElementId::Name(SharedString::from(format!(
                                        "habit-check-{id}"
                                    ))))
                                    .size(rems(3.))
                                    .rounded(theme.radius)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .map(|s| {
                                        if done {
                                            s.bg(theme.success)
                                                .text_color(theme.success_foreground)
                                                .child(Icon::new(Lucide::Check).small())
                                        } else if count > 0 {
                                            s.bg(theme.success.opacity(0.2))
                                                .border_1()
                                                .border_color(theme.success.opacity(0.6))
                                                .text_color(theme.foreground)
                                                .child(format!("{count}/{target}"))
                                        } else {
                                            s.bg(theme.muted)
                                                .border_1()
                                                .border_color(theme.border)
                                                .hover(|s| s.bg(theme.list_hover))
                                                .when(target > 1, |s| {
                                                    s.text_color(theme.muted_foreground)
                                                        .child(format!("{target}"))
                                                })
                                        }
                                    })
                                    .on_click(move |_, window, cx| {
                                        cx.stop_propagation();
                                        crate::views::task_row::notify_err(
                                            library.update(cx, |lib, cx| {
                                                lib.tap_habit(id, today, cx).map(|_| ())
                                            }),
                                            window,
                                            cx,
                                        );
                                    })
                                    .test_support(),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .text_center()
                                    .truncate()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(if done {
                                        theme.foreground
                                    } else {
                                        theme.muted_foreground
                                    })
                                    .child(h.habit.name().to_owned()),
                            )
                            .child(div().text_xs().text_color(theme.muted_foreground).child(
                                if streak > 0 {
                                    format!("{streak}d")
                                } else {
                                    caption(h, self.today)
                                },
                            ))
                    })))
                }
            })
            .into_any_element()
    }

    fn tasks_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let total = self.due.len();
        let meta = (total > 0).then(|| {
            if self.counts.overdue > 0 {
                format!("{total} · {} overdue", self.counts.overdue)
            } else {
                format!("{total}")
            }
        });
        let more = total.saturating_sub(TASKS_MAX);
        Self::card("tasks-card", cx)
            .child(self.title_row(
                "Due today",
                meta,
                Some(("today-all-tasks", "All tasks", Navigate::To(Route::Tasks))),
                cx,
            ))
            .map(|c| {
                if self.due.is_empty() {
                    c.child(Self::quiet(
                        "You're all caught up. Nothing is due today.",
                        cx,
                    ))
                } else {
                    c.child(v_flex().mx(px(-12.)).gap_0p5().children(
                        self.due.iter().take(TASKS_MAX).map(|t| {
                            task_row(
                                t,
                                self.today,
                                self.progress.get(&t.id()).copied(),
                                &self.library,
                                cx,
                            )
                        }),
                    ))
                    .when(more > 0, |c| {
                        c.child(
                            h_flex().child(
                                Button::new("today-more-tasks")
                                    .ghost()
                                    .xsmall()
                                    .label(format!("{more} more due"))
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(Navigate::To(Route::Tasks))
                                    })),
                            ),
                        )
                    })
                }
            })
            .into_any_element()
    }

    fn week_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let total: usize = self.week.iter().map(|(_, n)| n).sum();
        let today_label: SharedString = self.today.strftime("%a").to_string().into();
        let solid = theme.success;
        let calm = theme.success.opacity(0.48);
        let faint = theme.muted;
        let data: Vec<(SharedString, f64)> = self
            .week
            .iter()
            .map(|(day, n)| {
                (
                    SharedString::from(day.strftime("%a").to_string()),
                    *n as f64,
                )
            })
            .collect();
        let rated: Vec<f64> = self.habit_week.iter().flatten().copied().collect();
        let habit_avg = (!rated.is_empty()).then(|| rated.iter().sum::<f64>() / rated.len() as f64);
        let has_tasks = total > 0;
        Self::card("week-card", cx)
            .child(self.title_row(
                "This week",
                Some(match habit_avg {
                    Some(a) => {
                        format!("{total} tasks done · habits {}%", (a * 100.).round() as i64)
                    }
                    None => format!("{total} tasks done"),
                }),
                None,
                cx,
            ))
            .child(
                div().id("week-chart").h(rems(6.5)).test_support().child(
                    BarChart::new(data)
                        .band(|d: &(SharedString, f64)| d.0.clone())
                        .value(move |d: &(SharedString, f64)| {
                            // A sliver for empty days keeps the week's shape.
                            if has_tasks { d.1.max(0.05) } else { 0.05 }
                        })
                        .fill(move |d: &(SharedString, f64), _, _, _| {
                            if d.1 <= 0. {
                                faint
                            } else if d.0 == today_label {
                                solid
                            } else {
                                calm
                            }
                        })
                        .max_band_width(px(22.))
                        .corner_radii(Corners {
                            top_left: px(3.),
                            top_right: px(3.),
                            ..Default::default()
                        })
                        .grid(false),
                ),
            )
            .when(!self.habits.is_empty(), |c| {
                // Habit days under the bars: fill = share of habits kept.
                c.child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("Habits kept"),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .children(self.habit_week.iter().zip(&self.week).map(
                                    |(rate, (day, _))| {
                                        let label = match rate {
                                            Some(r) => format!("{}%", (r * 100.).round() as i64),
                                            None => "—".into(),
                                        };
                                        v_flex()
                                            .flex_1()
                                            .gap_0p5()
                                            .items_center()
                                            .child(
                                                div().h(rems(0.375)).w_full().rounded_full().bg(
                                                    match rate {
                                                        Some(r) if *r > 0. => theme
                                                            .success
                                                            .opacity((0.3 + 0.7 * *r) as f32),
                                                        _ => theme.muted,
                                                    },
                                                ),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(if *day == self.today {
                                                        theme.foreground
                                                    } else {
                                                        theme.muted_foreground
                                                    })
                                                    .child(label),
                                            )
                                    },
                                )),
                        ),
                )
            })
            .into_any_element()
    }

    fn upcoming_items(&self) -> Vec<(Date, Upcoming)> {
        let mut items: Vec<(Date, Upcoming)> = Vec::new();
        for occ in &self.span.occurrences {
            let day = occ.start.date().max(self.today);
            if day > self.today && !self.is_routine(occ) {
                items.push((day, Upcoming::Event(*occ)));
            }
        }
        let horizon = add_days(self.today, UPCOMING_DAYS);
        for t in &self.span.tasks {
            if let Some(due) = t.due()
                && due > self.today
                && due <= horizon
                && t.status().is_open()
            {
                items.push((due, Upcoming::Task(t.clone())));
            }
        }
        items.sort_by_key(|(d, item)| {
            (
                *d,
                match item {
                    Upcoming::Event(o) => (0, o.all_day, Some(o.start)),
                    Upcoming::Task(_) => (1, false, None),
                },
            )
        });
        items.truncate(UPCOMING_MAX);
        items
    }

    fn upcoming_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let clock = appearance::settings(cx).clock;
        let items = self.upcoming_items();
        let mut rows: Vec<AnyElement> = Vec::new();
        // A day heading names its holidays ("Tomorrow · Diwali"); a holiday
        // with nothing else planned still gets its heading.
        let heading = |day: Date, rows: &mut Vec<AnyElement>| {
            let label = match self.holidays.summary(day) {
                Some(h) => format!("{} · {h}", relative_day(day, self.today)),
                None => relative_day(day, self.today),
            };
            rows.push(
                div()
                    .pt_1p5()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(label)
                    .into_any_element(),
            );
        };
        let item_days: Vec<Date> = items.iter().map(|(d, _)| *d).collect();
        let last_item_day = item_days.last().copied();
        let mut holiday_days = self
            .holidays
            .days()
            .filter(|d| *d > self.today && !item_days.contains(d))
            // When the list was cut short, don't add days beyond it.
            .filter(|d| items.len() < UPCOMING_MAX || last_item_day.is_some_and(|l| *d <= l))
            .peekable();
        let mut last_day = None;
        for (day, item) in &items {
            while let Some(h) = holiday_days.next_if(|h| h < day) {
                heading(h, &mut rows);
            }
            if last_day != Some(*day) {
                last_day = Some(*day);
                heading(*day, &mut rows);
            }
            let (key, icon, when, title, on_click): (String, Lucide, String, String, ClickFn) =
                match item {
                    Upcoming::Event(o) => {
                        let id = o.event;
                        let series = o.series_date;
                        let library = self.library.clone();
                        (
                            format!("upcoming-event-{id}-{day}"),
                            self.occasion_icon(o).unwrap_or(Lucide::CalendarDays),
                            if o.all_day {
                                "All day".into()
                            } else {
                                format_time(o.start.time(), clock)
                            },
                            self.title_of(o),
                            Box::new(move |_, window, cx| {
                                open_event_detail(
                                    id,
                                    Some(series),
                                    false,
                                    library.clone(),
                                    window,
                                    cx,
                                )
                            }),
                        )
                    }
                    Upcoming::Task(t) => {
                        let id = t.id();
                        let library = self.library.clone();
                        (
                            format!("upcoming-{id}"),
                            Lucide::Circle,
                            "Task".into(),
                            t.title().to_owned(),
                            Box::new(move |_, window, cx| {
                                open_task_detail(id, library.clone(), window, cx)
                            }),
                        )
                    }
                };
            rows.push(
                h_flex()
                    .id(ElementId::Name(key.into()))
                    .gap_2()
                    .mx(px(-8.))
                    .px(px(8.))
                    .py_1()
                    .items_center()
                    .rounded(theme.radius)
                    .text_sm()
                    .hover(|s| s.bg(theme.list_hover))
                    .on_click(on_click)
                    .test_support()
                    .child(
                        div()
                            .w(rems(4.5))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(when),
                    )
                    .child(Icon::new(icon).xsmall().text_color(theme.muted_foreground))
                    .child(div().flex_1().min_w_0().truncate().child(title))
                    .into_any_element(),
            );
        }
        for h in holiday_days {
            heading(h, &mut rows);
        }
        Self::card("upcoming-card", cx)
            .child(self.title_row(
                "Next 7 days",
                None,
                Some((
                    "today-upcoming-calendar",
                    "Calendar",
                    Navigate::To(Route::Calendar),
                )),
                cx,
            ))
            .map(|c| {
                if rows.is_empty() {
                    c.child(Self::quiet("A clear week ahead.", cx))
                } else {
                    c.child(v_flex().children(rows))
                }
            })
            .into_any_element()
    }

    fn notes_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        Self::card("notes-card", cx)
            .child(self.title_row(
                "Recent notes",
                None,
                Some(("today-all-notes", "All notes", Navigate::To(Route::Notes))),
                cx,
            ))
            .map(|c| {
                if self.recent.is_empty() {
                    c.child(Self::quiet(
                        if cfg!(target_os = "macos") {
                            "No notes yet. Press ⌘N to write one."
                        } else {
                            "No notes yet. Press Ctrl+N to write one."
                        },
                        cx,
                    ))
                } else {
                    c.child(
                        h_flex()
                            .flex_wrap()
                            .gap_3()
                            .children(self.recent.iter().map(|n| {
                                let id = n.id;
                                v_flex()
                                    .id(ElementId::Name(SharedString::from(format!("recent-{id}"))))
                                    .flex_1()
                                    .min_w(rems(13.))
                                    .gap_1()
                                    .p_3()
                                    .rounded(theme.radius)
                                    .bg(theme.background)
                                    .border_1()
                                    .border_color(theme.border)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme.list_hover))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(Navigate::OpenNote(id))
                                    }))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(n.title.clone()),
                                    )
                                    .when(!n.preview.is_empty(), |s| {
                                        s.child(
                                            div()
                                                .truncate()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child(n.preview.clone()),
                                        )
                                    })
                            })),
                    )
                }
            })
            .into_any_element()
    }

    fn section(&self, s: TodaySection, cx: &mut Context<Self>) -> AnyElement {
        // Wrapped so tests can find any section by `today-<key>`.
        let id = SharedString::from(format!("today-{}", s.key()));
        let body = match s {
            TodaySection::Glance => self.glance(cx),
            TodaySection::Agenda => self.agenda(cx),
            TodaySection::Habits => self.habits_section(cx),
            TodaySection::Tasks => self.tasks_section(cx),
            TodaySection::Week => self.week_section(cx),
            TodaySection::Upcoming => self.upcoming_section(cx),
            TodaySection::Notes => self.notes_section(cx),
        };
        div().id(id).child(body).test_support().into_any_element()
    }

    /// "Customize" menu: show or hide each section; reorder in Settings.
    fn customize(cx: &App) -> impl IntoElement {
        let prefs = appearance::settings(cx);
        let sections = today_layout::resolve(&prefs.today_order, &prefs.today_hidden);
        Button::new("today-customize")
            .ghost()
            .small()
            .icon(Lucide::SlidersHorizontal)
            .tooltip("Customize Today")
            .dropdown_menu(move |menu, _, _| {
                let menu = menu.label("Show on Today");
                sections
                    .iter()
                    .fold(menu, |menu, (section, shown)| {
                        let (section, shown) = (*section, *shown);
                        menu.item(
                            PopupMenuItem::new(section.title())
                                .checked(shown)
                                .on_click(move |_, _, cx| set_shown(section, !shown, cx)),
                        )
                    })
                    .separator()
                    .item(
                        PopupMenuItem::new("Reorder sections…").on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(crate::actions::GoSettings), cx)
                        }),
                    )
            })
    }
}

/// Shows or hides a section (persisted with the appearance settings).
pub fn set_shown(section: TodaySection, shown: bool, cx: &mut App) {
    appearance::update(cx, |s| {
        s.today_hidden.retain(|k| k != section.key());
        if !shown {
            s.today_hidden.push(section.key().to_owned());
        }
    });
}

impl Focusable for TodayView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TodayView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut date_label = self.today.strftime("%A, %B %-d").to_string();
        if let Some(h) = self.holidays.summary(self.today) {
            date_label = format!("{date_label} · {h}");
        }
        let prefs = appearance::settings(cx);
        let visible: Vec<TodaySection> =
            today_layout::resolve(&prefs.today_order, &prefs.today_hidden)
                .into_iter()
                .filter(|(_, shown)| *shown)
                .map(|(s, _)| s)
                .collect();

        let event_count = self.today_events().len();
        let task_left = self.counts.due_today + self.counts.overdue;
        let habit_due = self.habits.iter().filter(|h| h.due_today).count();
        let habit_done = self
            .habits
            .iter()
            .filter(|h| h.due_today && h.done_today())
            .count();
        let summary = format!(
            "{event_count} {} · {task_left} {} · {habit_done}/{habit_due} habits",
            if event_count == 1 { "event" } else { "events" },
            if task_left == 1 { "task" } else { "tasks" },
        );
        let sync_pill = {
            let lib = self.library.read(cx);
            let signed_in = lib.account().signed_in_email().is_some();
            if signed_in {
                let label = if lib.sync_in_flight() {
                    "Syncing…"
                } else if lib.sync_last_error().is_some() {
                    "Sync issue"
                } else if lib
                    .sync_state()
                    .ok()
                    .and_then(|s| s.last_synced_at)
                    .is_some()
                {
                    "Synced"
                } else {
                    "Cloud"
                };
                let dot = if lib.sync_last_error().is_some() {
                    theme.warning
                } else {
                    theme.success
                };
                Some(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .px_2p5()
                        .py_0p5()
                        .rounded_full()
                        .bg(theme.group_box)
                        .border_1()
                        .border_color(theme.border)
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(div().size_1p5().rounded_full().bg(dot))
                        .child(label),
                )
            } else {
                None
            }
        };

        let header = h_flex()
            .justify_between()
            .items_end()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(date_label),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(self.greeting()),
                            )
                            .children(sync_pill),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(summary),
                    ),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(
                        Button::new("today-daily-note")
                            .primary()
                            .small()
                            .icon(Lucide::NotebookPen)
                            .label(if self.has_daily_note {
                                "Open today's note"
                            } else {
                                "Today's note"
                            })
                            .tooltip_with_action(
                                "Today's note",
                                &crate::actions::OpenDailyNote,
                                None,
                            )
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(OpenDailyNoteRequest))),
                    )
                    .child(Self::customize(cx)),
            );

        let rows: Vec<AnyElement> =
            today_layout::rows(&visible)
                .into_iter()
                .map(|row| {
                    if row.len() == 1 {
                        self.section(row[0], cx)
                    } else {
                        h_flex()
                            .flex_wrap()
                            .items_start()
                            .gap_4()
                            .children(row.into_iter().map(|s| {
                                div().flex_1().min_w(rems(20.)).child(self.section(s, cx))
                            }))
                            .into_any_element()
                    }
                })
                .collect();

        div()
            .id("today-scroll")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .w_full()
                    .max_w(rems(56.))
                    .mx_auto()
                    .px_6()
                    .py_8()
                    .gap_4()
                    .child(div().pb_2().child(header))
                    .children(rows)
                    .when(visible.is_empty(), |s| {
                        s.child(
                            div()
                                .py_8()
                                .text_sm()
                                .text_center()
                                .text_color(theme.muted_foreground)
                                .child(
                                    "Every section is hidden. Use Customize to bring some back.",
                                ),
                        )
                    }),
            )
    }
}
