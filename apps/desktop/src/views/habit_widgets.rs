//! Habit pieces shared by the Habits screen, the habit panel and Today:
//! the round check, the 7-day strip and the history heatmap.
//!
//! One visual language everywhere, kept quiet so a list of habits doesn't
//! turn into a wall of check marks: done = solid dot, partly done = faint
//! dot, missed = hollow ring, not expected = pinpoint. Only today's check is
//! a real control-sized circle.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div, rems,
};
use jiff::Span;
use ottero_core::Date;
use ottero_core::habit::Schedule;

use crate::library::{HabitSummary, Library};
use crate::views::task_row::notify_err;

/// Days in the strip.
pub const STRIP_DAYS: i64 = 7;
/// Width of one strip cell (also used by the column header).
pub const CELL: f32 = 1.5;

pub fn add_days(day: Date, n: i64) -> Date {
    day.checked_add(Span::new().days(n)).unwrap_or(day)
}

/// The strip's days, oldest first: the week before today (today has its
/// own check column, so it isn't repeated here).
pub fn strip_days(today: Date) -> Vec<Date> {
    (1..=STRIP_DAYS)
        .rev()
        .map(|k| add_days(today, -k))
        .collect()
}

/// What a day looks like for a habit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayState {
    Done,
    Partial(u16),
    Missed,
    /// Not expected that day (or a weekly goal's free day): no mark.
    Rest,
}

pub fn day_state(h: &HabitSummary, day: Date, today: Date) -> DayState {
    let count = h.count_on(day);
    let target = h.habit.target();
    if count >= target {
        DayState::Done
    } else if count > 0 {
        DayState::Partial(count)
    } else if day < h.first
        || day >= today
        || !h.habit.schedule().is_due(day)
        || matches!(h.habit.schedule(), Schedule::PerWeek(_))
    {
        DayState::Rest
    } else {
        DayState::Missed
    }
}

/// The round check for today: tap to count up / toggle.
pub fn check_button(
    h: &HabitSummary,
    today: Date,
    library: &Entity<Library>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let id = h.habit.id();
    let target = h.habit.target();
    let count = h.today_count;
    let done = count >= target;
    let library = library.clone();
    let tip: SharedString = match (done, target) {
        (true, 1) => "Done today. Click to undo.".into(),
        (true, _) => format!("{target} of {target} today. Click to reset.").into(),
        (false, 1) => "Mark done for today".into(),
        (false, _) => format!("{count} of {target} today. Click to add one.").into(),
    };
    let ring = theme.muted_foreground.opacity(0.45);
    let hover_ring = theme.foreground.opacity(0.7);
    div()
        .id(SharedString::from(format!("habit-check-{id}")))
        .flex_shrink_0()
        .size(rems(CHECK))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .map(|s| {
            if done {
                s.bg(theme.success)
                    .text_color(theme.success_foreground)
                    .child(Icon::new(Lucide::Check).xsmall())
            } else if count > 0 {
                s.border_1()
                    .border_color(theme.success.opacity(0.6))
                    .bg(theme.success.opacity(0.14))
                    .child(format!("{count}/{target}"))
            } else {
                s.border_1()
                    .border_color(ring)
                    .hover(move |s| s.border_color(hover_ring))
                    .when(target > 1, |s| {
                        s.text_color(theme.muted_foreground)
                            .child(format!("{target}"))
                    })
            }
        })
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            notify_err(
                library.update(cx, |lib, cx| lib.tap_habit(id, today, cx).map(|_| ())),
                window,
                cx,
            );
        })
        .test_support()
}

/// Size of the today check, in rems (the column header matches it).
pub const CHECK: f32 = 1.75;

/// Weekday initials over the strip.
pub fn strip_header(today: Date, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    h_flex().children(strip_days(today).into_iter().map(|day| {
        div()
            .w(rems(CELL))
            .text_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(
                day.strftime("%a")
                    .to_string()
                    .chars()
                    .next()
                    .unwrap_or(' ')
                    .to_string(),
            )
    }))
}

/// The week before today as dots; click one to check in late (backfill).
pub fn week_strip(
    h: &HabitSummary,
    today: Date,
    library: &Entity<Library>,
    cx: &App,
) -> impl IntoElement {
    let id = h.habit.id();
    h_flex().children(strip_days(today).into_iter().map(|day| {
        let state = day_state(h, day, today);
        let library = library.clone();
        let label: SharedString = format!(
            "{} · {}",
            day.strftime("%a, %b %-d"),
            match state {
                DayState::Done => "Done".to_owned(),
                DayState::Partial(c) => format!("{c} of {}", h.habit.target()),
                DayState::Missed => "Missed".to_owned(),
                DayState::Rest if !h.habit.schedule().is_due(day) => "Not scheduled".to_owned(),
                DayState::Rest => "Not done".to_owned(),
            }
        )
        .into();
        div()
            .id(SharedString::from(format!("habit-{id}-{day}")))
            .w(rems(CELL))
            .h(rems(CELL))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .hover(|s| s.bg(cx.theme().list_hover))
            .child(day_mark(state, cx))
            .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                notify_err(
                    library.update(cx, |lib, cx| lib.tap_habit(id, day, cx).map(|_| ())),
                    window,
                    cx,
                );
            })
            .into_any_element()
    }))
}

/// Done: solid dot. Partly: half-strength dot. Missed: hollow ring.
/// Rest: a faint pinpoint. Shapes differ, so color is never the only cue.
fn day_mark(state: DayState, cx: &App) -> gpui_kit::Div {
    let theme = cx.theme();
    let dot = rems(0.625);
    match state {
        DayState::Done => div().size(dot).rounded_full().bg(theme.primary),
        DayState::Partial(_) => div()
            .size(dot)
            .rounded_full()
            .bg(theme.primary.opacity(0.4)),
        DayState::Missed => div()
            .size(dot)
            .rounded_full()
            .border_1()
            .border_color(theme.muted_foreground.opacity(0.5)),
        DayState::Rest => div()
            .size(rems(0.1875))
            .rounded_full()
            .bg(theme.muted_foreground.opacity(0.35)),
    }
}

/// History grid: one column per week (Monday on top), newest on the right,
/// with month labels. Read-only; the strip is where you check in.
pub fn heatmap(h: &HabitSummary, today: Date, weeks: i64, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let monday = add_days(today, -i64::from(today.weekday().to_monday_zero_offset()));
    let first_week = add_days(monday, -7 * (weeks - 1));
    let target = h.habit.target();
    let cell = rems(0.75);
    let columns = (0..weeks).map(|w| {
        let week = add_days(first_week, 7 * w);
        let month_label = (week.day() <= 7 || w == 0).then(|| week.strftime("%b").to_string());
        v_flex()
            .gap(rems(0.1875))
            .child(
                div()
                    .h(rems(1.))
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .whitespace_nowrap()
                    .children(month_label),
            )
            .children((0..7).map(|i| {
                let day = add_days(week, i);
                let count = h.count_on(day);
                let fill = if day > today {
                    None
                } else if count >= target {
                    Some(theme.primary)
                } else if count > 0 {
                    Some(theme.primary.opacity(0.35))
                } else if day >= h.first && h.habit.schedule().is_due(day) {
                    Some(theme.muted)
                } else {
                    Some(theme.muted.opacity(0.4))
                };
                div()
                    .size(cell)
                    .rounded(rems(0.1875))
                    .when_some(fill, |s, c| s.bg(c))
            }))
    });
    let legend_cell = |c: gpui_kit::Hsla| div().size(cell).rounded(rems(0.1875)).bg(c);
    v_flex()
        .gap_2()
        .child(
            h_flex()
                .gap(rems(0.1875))
                .child(
                    v_flex()
                        .gap(rems(0.1875))
                        .pr_1()
                        .child(div().h(rems(1.)))
                        .children(["M", "", "W", "", "F", "", ""].into_iter().map(|l| {
                            div()
                                .h(cell)
                                .text_xs()
                                .line_height(cell)
                                .text_color(theme.muted_foreground)
                                .child(l)
                        })),
                )
                .children(columns),
        )
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("Missed")
                .child(legend_cell(theme.muted))
                .child(legend_cell(theme.primary.opacity(0.35)))
                .child(legend_cell(theme.primary))
                .child("Done"),
        )
        .into_any_element()
}

/// "Every day", "Mon, Wed, Fri", "2 of 3 this week", plus the goal.
pub fn caption(h: &HabitSummary, today: Date) -> String {
    let mut parts = vec![match h.habit.schedule() {
        Schedule::PerWeek(n) => format!(
            "{} of {n} this week",
            h.done_this_week(today).min(usize::from(n))
        ),
        other => other.to_string(),
    }];
    if h.habit.target() > 1 {
        parts.push(format!("{} times a day", h.habit.target()));
    }
    parts.join(" · ")
}
