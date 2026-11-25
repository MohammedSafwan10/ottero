//! Habits screen with three tabs:
//!
//! * **Today**: quick add, then one card of habits (scheduled today first,
//!   the rest under "Not today"). Each row: name and schedule, the past
//!   week as dots, the streak, and today's check.
//! * **Insights**: analytics across habits (`habit_insights_view`).
//! * **Archived**: paused habits, history kept.
//!
//! State: the active and archived lists (from `Library::habits`), the tab,
//! and the keyboard selection (an index into the Today order). Reloads on
//! `HabitsChanged` and when the window comes back (via `reload`).
//!
//! Keyboard (list focused): ↑/↓ select, Space checks in today, Enter opens,
//! N types a new habit. Rows drag to reorder.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window, div,
    rems,
};
use ottero_core::habit::StreakUnit;
use ottero_core::{Date, HabitId, clock};
use ottero_ui::components::{empty_state, page_header, segmented};

use crate::actions::{
    HABITS_CONTEXT, HabitOpenSelected, HabitQuickAdd, HabitSelectNext, HabitSelectPrevious,
    HabitToggleSelected,
};
use crate::library::{HabitSummary, Library, LibraryEvent};
use crate::views::habit_detail::open_habit_detail;
use crate::views::habit_widgets::{caption, check_button, strip_header, week_strip};
use crate::views::task_row::notify_err;

/// Width of the streak column, in rems.
const STREAK_W: f32 = 4.5;
/// Width of the today lane (header label and check), in rems.
const TODAY_W: f32 = 3.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HabitTab {
    Today,
    Insights,
    Archived,
}

/// Dragged row, for reordering.
#[derive(Clone)]
pub struct HabitDrag {
    id: HabitId,
    name: SharedString,
}

impl Render for HabitDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_3()
            .py_1p5()
            .rounded(theme.radius)
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .shadow_md()
            .text_sm()
            .child(self.name.clone())
    }
}

pub struct HabitsView {
    pub(super) library: Entity<Library>,
    input: Entity<InputState>,
    hint: Option<SharedString>,
    tab: HabitTab,
    pub(super) habits: Vec<HabitSummary>,
    archived: Vec<HabitSummary>,
    /// Index into `ordered()`.
    selected: Option<usize>,
    load_error: bool,
    pub(super) today: Date,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl HabitsView {
    pub fn new(library: Entity<Library>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Add a habit and press Enter"));
        let subscriptions = vec![
            cx.subscribe(&library, |this, _, event, cx| {
                if *event == LibraryEvent::HabitsChanged {
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
                        this.hint = hint(&text);
                        cx.notify();
                    }
                    _ => {}
                },
            ),
        ];
        let mut this = Self {
            library,
            input,
            hint: None,
            tab: HabitTab::Today,
            habits: Vec::new(),
            archived: Vec::new(),
            selected: None,
            load_error: false,
            today: clock::today(),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.today = clock::today();
        let lib = self.library.read(cx);
        match (lib.habits(false), lib.habits(true)) {
            (Ok(active), Ok(archived)) => {
                self.habits = active;
                self.archived = archived;
                self.load_error = false;
            }
            (a, b) => {
                if let Err(err) = a.and(b) {
                    eprintln!("ottero: failed to load habits: {err:#}");
                }
                self.habits.clear();
                self.archived.clear();
                self.load_error = true;
            }
        }
        let n = self.shown().len();
        self.selected = self.selected.filter(|_| n > 0).map(|i| i.min(n - 1));
        cx.notify();
    }

    /// The list the current tab shows (Insights uses the active list).
    pub fn habits(&self) -> &[HabitSummary] {
        self.shown()
    }

    fn shown(&self) -> &[HabitSummary] {
        match self.tab {
            HabitTab::Archived => &self.archived,
            _ => &self.habits,
        }
    }

    pub fn tab(&self) -> HabitTab {
        self.tab
    }

    pub fn set_tab(&mut self, tab: HabitTab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            self.selected = None;
            cx.notify();
        }
    }

    /// Kept for callers and tests: `true` shows the Archived tab.
    pub fn set_archived(&mut self, archived: bool, cx: &mut Context<Self>) {
        self.set_tab(
            if archived {
                HabitTab::Archived
            } else {
                HabitTab::Today
            },
            cx,
        );
    }

    /// Today's habits first, then the rest, each in list order.
    fn ordered(&self) -> Vec<&HabitSummary> {
        let archived = self.tab == HabitTab::Archived;
        let (today, other): (Vec<_>, Vec<_>) =
            self.shown().iter().partition(|h| archived || h.due_today);
        today.into_iter().chain(other).collect()
    }

    pub fn selected_habit(&self) -> Option<HabitId> {
        self.selected
            .and_then(|i| self.ordered().get(i).map(|h| h.habit.id()))
    }

    pub fn focus_quick_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_tab(HabitTab::Today, cx);
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn add_from_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let result = self
            .library
            .update(cx, |lib, cx| lib.quick_add_habit(&text, cx));
        if result.is_ok() {
            self.input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.hint = None;
        }
        notify_err(result, window, cx);
    }

    fn step_selection(&mut self, dir: i64, cx: &mut Context<Self>) {
        let n = self.shown().len();
        if n == 0 || self.tab == HabitTab::Insights {
            return;
        }
        let next = match self.selected {
            None if dir > 0 => 0,
            None => n - 1,
            Some(i) => (i as i64 + dir).clamp(0, n as i64 - 1) as usize,
        };
        self.selected = Some(next);
        cx.notify();
    }

    fn toggle_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(id), HabitTab::Today) = (self.selected_habit(), self.tab) else {
            return;
        };
        let today = self.today;
        notify_err(
            self.library
                .update(cx, |lib, cx| lib.tap_habit(id, today, cx).map(|_| ())),
            window,
            cx,
        );
    }

    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_habit() {
            open_habit_detail(id, self.library.clone(), window, cx);
        }
    }

    fn row(
        &self,
        index: usize,
        h: &HabitSummary,
        last: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let id = h.habit.id();
        let selected = self.selected == Some(index);
        let archived = self.tab == HabitTab::Archived;
        let muted = !h.due_today && !archived;
        let streak = h.stats.current_streak;
        let streak_label: SharedString = match (h.stats.streak_unit, streak) {
            (_, 0) => "—".into(),
            (StreakUnit::Days, 1) => "1 day".into(),
            (StreakUnit::Days, n) => format!("{n} days").into(),
            (StreakUnit::Weeks, 1) => "1 week".into(),
            (StreakUnit::Weeks, n) => format!("{n} weeks").into(),
        };
        h_flex()
            .id(SharedString::from(format!("habit-{id}")))
            .w_full()
            .gap_4()
            .px_4()
            .py_3()
            .items_center()
            .when(!last, |s| s.border_b_1().border_color(theme.border))
            .map(|s| {
                if selected {
                    s.bg(theme.list_active)
                } else {
                    s.hover(|s| s.bg(theme.list_hover))
                }
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = Some(index);
                window.focus(&this.focus, cx);
                open_habit_detail(id, this.library.clone(), window, cx);
            }))
            .when(!archived, |s| {
                s.on_drag(
                    HabitDrag {
                        id,
                        name: h.habit.name().to_owned().into(),
                    },
                    |drag, _, _, cx| cx.new(|_| drag.clone()),
                )
                .drag_over::<HabitDrag>(|s, _, _, cx| s.bg(cx.theme().drop_target))
                .on_drop(cx.listener(move |this, drag: &HabitDrag, window, cx| {
                    // Drop position in display order → index in the stored order.
                    let target = this.ordered().get(index).map(|h| h.habit.id());
                    let to = target
                        .and_then(|t| this.habits.iter().position(|h| h.habit.id() == t))
                        .unwrap_or(0);
                    let moved = drag.id;
                    notify_err(
                        this.library
                            .update(cx, |lib, cx| lib.move_habit(moved, to, cx)),
                        window,
                        cx,
                    );
                }))
            })
            .test_support()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .when(muted, |s| s.text_color(theme.muted_foreground))
                            .child(h.habit.name().to_owned()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(caption(h, self.today)),
                    ),
            )
            .child(week_strip(h, self.today, &self.library, cx))
            .child(
                h_flex()
                    .w(rems(STREAK_W))
                    .flex_shrink_0()
                    .gap_1()
                    .justify_end()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .when(streak > 0, |s| {
                        s.text_color(theme.foreground)
                            .child(Icon::new(Lucide::Flame).xsmall())
                    })
                    .child(streak_label),
            )
            .child(
                h_flex()
                    .w(rems(TODAY_W))
                    .flex_shrink_0()
                    .justify_center()
                    .when(!archived, |s| {
                        s.child(check_button(h, self.today, &self.library, cx))
                    }),
            )
            .into_any_element()
    }

    /// Column labels, aligned with the row lanes (same padding and gaps).
    fn columns(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let archived = self.tab == HabitTab::Archived;
        h_flex()
            .gap_4()
            .px_4()
            .py_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(
                div()
                    .flex_1()
                    .child(if archived { "Archived" } else { "Habit" }),
            )
            .child(strip_header(self.today, cx))
            .child(
                div()
                    .w(rems(STREAK_W))
                    .flex_shrink_0()
                    .text_right()
                    .child("Streak"),
            )
            .child(
                div()
                    .w(rems(TODAY_W))
                    .flex_shrink_0()
                    .text_center()
                    .when(!archived, |s| s.child("Today")),
            )
    }

    fn group_label(text: &'static str, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_4()
            .pt_3()
            .pb_1()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.muted.opacity(0.35))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.muted_foreground)
            .child(text)
    }

    fn list(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let archived = self.tab == HabitTab::Archived;
        if self.shown().is_empty() {
            let (title, desc) = if self.load_error {
                (
                    "Couldn't load habits",
                    "Your habits are still on disk. Restart Ottero; if this keeps happening, check the data folder in Settings.",
                )
            } else if archived {
                (
                    "No archived habits",
                    "Archive a habit to pause it without losing its history.",
                )
            } else {
                (
                    "Start a habit",
                    "Add one above, like “Read 20 pages daily” or “Run 3x a week”. Check it off each day to build a streak.",
                )
            };
            return empty_state(Lucide::Sprout, title, desc, cx).into_any_element();
        }
        let ordered: Vec<HabitSummary> = self.ordered().into_iter().cloned().collect();
        let first_other = ordered.iter().position(|h| !h.due_today && !archived);
        let mut rows: Vec<AnyElement> = Vec::new();
        for (i, h) in ordered.iter().enumerate() {
            if Some(i) == first_other {
                rows.push(Self::group_label("Not today", cx).into_any_element());
            }
            rows.push(self.row(i, h, i + 1 == ordered.len(), cx));
        }
        v_flex()
            .bg(theme.group_box)
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .when(theme.shadow, |s| s.shadow_xs())
            .overflow_hidden()
            .child(self.columns(cx))
            .children(rows)
            .into_any_element()
    }
}

fn hint(text: &str) -> Option<SharedString> {
    if text.trim().is_empty() {
        return None;
    }
    let draft = ottero_core::parse_habit(text);
    let mut parts = vec![draft.schedule.to_string()];
    if draft.target > 1 {
        parts.push(format!("{} times a day", draft.target));
    }
    Some(format!("{} — {}", draft.name, parts.join(" · ")).into())
}

impl Focusable for HabitsView {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for HabitsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let due: Vec<&HabitSummary> = self.habits.iter().filter(|h| h.due_today).collect();
        let done = due.iter().filter(|h| h.done_today()).count();
        let subtitle: Option<SharedString> = match self.tab {
            HabitTab::Today if self.habits.is_empty() => None,
            HabitTab::Today if due.is_empty() => Some("Nothing scheduled today.".into()),
            HabitTab::Today if done == due.len() => {
                Some(format!("All {} done today", due.len()).into())
            }
            HabitTab::Today => Some(format!("{done} of {} done today", due.len()).into()),
            HabitTab::Insights => Some("How your habits are going".into()),
            HabitTab::Archived => Some("Paused habits keep their history".into()),
        };
        let view = cx.entity().downgrade();
        let tabs = segmented(
            "habit-tabs",
            &[
                (HabitTab::Today, "Today"),
                (HabitTab::Insights, "Insights"),
                (HabitTab::Archived, "Archived"),
            ],
            self.tab,
            move |tab, _, cx| {
                _ = view.update(cx, |this, cx| this.set_tab(tab, cx));
            },
        );
        // Today's progress as a hairline under the header.
        let progress = (self.tab == HabitTab::Today && !due.is_empty()).then(|| {
            let share = done as f32 / due.len() as f32;
            div()
                .h(rems(0.25))
                .w_full()
                .rounded_full()
                .bg(theme.muted)
                .child(
                    div()
                        .h_full()
                        .w(gpui_kit::relative(share))
                        .rounded_full()
                        .bg(theme.success),
                )
        });

        let body = match self.tab {
            HabitTab::Insights => self.insights(cx),
            _ => self.list(cx),
        };

        v_flex()
            .size_full()
            .child(
                v_flex()
                    .w_full()
                    .max_w(rems(52.))
                    .mx_auto()
                    .px_6()
                    .pt_6()
                    .gap_4()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_end()
                            .gap_4()
                            .child(page_header("Habits", subtitle, cx))
                            .child(tabs),
                    )
                    .children(progress)
                    .when(self.tab == HabitTab::Today, |this| {
                        this.child(
                            v_flex()
                                .gap_1()
                                .child(Input::new(&self.input).id("habit-input").prefix(
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
                                            "Try “Read 20 pages daily”, “Gym mon wed fri”, “Run 3x a week”, “Water 8 times a day”".into()
                                        })),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .id("habit-list")
                    .track_focus(&self.focus)
                    .key_context(HABITS_CONTEXT)
                    .on_action(cx.listener(|this, _: &HabitSelectPrevious, _, cx| {
                        this.step_selection(-1, cx)
                    }))
                    .on_action(
                        cx.listener(|this, _: &HabitSelectNext, _, cx| this.step_selection(1, cx)),
                    )
                    .on_action(cx.listener(|this, _: &HabitToggleSelected, window, cx| {
                        this.toggle_selected(window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &HabitOpenSelected, window, cx| {
                        this.open_selected(window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &HabitQuickAdd, window, cx| {
                        this.focus_quick_add(window, cx)
                    }))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .w_full()
                            .max_w(rems(52.))
                            .mx_auto()
                            .px_6()
                            .pt_2()
                            .pb_8()
                            .child(body),
                    ),
            )
    }
}
