//! Focus screen: calm countdown ring, optional task link, and today's session log.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::combobox::{Combobox, ComboboxEvent, ComboboxState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::ProgressCircle;
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window, div,
    rems,
};
use ottero_core::focus::{MAX_DURATION_MINS, MIN_DURATION_MINS};
use ottero_core::{
    Date, FOCUS_DURATION_PRESETS, FocusKind, FocusSession, FocusSessionId, Task, TaskStatus, clock,
    format_duration, format_mm_ss,
};
use ottero_storage::TaskFilter;
use ottero_ui::components::{empty_state, page_header, section_label};
use ottero_ui::feedback;

use crate::actions::FOCUS_CONTEXT;
use crate::focus::{FocusController, FocusEvent};
use crate::library::{Library, LibraryEvent};
use crate::views::task_detail::open_task_detail;
use crate::views::task_row::notify_err;

/// Stable IDs keep duplicate task titles distinct in the searchable picker.
#[derive(Clone)]
struct FocusTaskItem {
    id: ottero_core::TaskId,
    title: SharedString,
}

impl SearchableListItem for FocusTaskItem {
    type Value = ottero_core::TaskId;
    fn title(&self) -> SharedString {
        self.title.clone()
    }
    fn value(&self) -> &Self::Value {
        &self.id
    }
    fn matches(&self, query: &str) -> bool {
        self.title
            .to_lowercase()
            .contains(&query.trim().to_lowercase())
    }
}

pub struct FocusView {
    library: Entity<Library>,
    focus: Entity<FocusController>,
    sessions_today: Vec<FocusSession>,
    open_tasks: Vec<Task>,
    load_error: bool,
    tasks_error: bool,
    tasks_dirty: bool,
    task_picker: Entity<ComboboxState<SearchableVec<FocusTaskItem>>>,
    editing_custom: bool,
    custom_input: Entity<InputState>,
    today: Date,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl FocusView {
    pub fn new(
        library: Entity<Library>,
        focus: Entity<FocusController>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let custom_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("min")
                .validate(|text, _| text.len() <= 3 && text.chars().all(|c| c.is_ascii_digit()))
        });
        let task_picker = cx.new(|cx| {
            ComboboxState::new(
                SearchableVec::<FocusTaskItem>::new(vec![]),
                vec![],
                window,
                cx,
            )
            .searchable(true)
        });
        let subscriptions = vec![
            cx.subscribe(&task_picker, |this, _, event, cx| {
                if let ComboboxEvent::Change(ids) = event {
                    let task = ids
                        .first()
                        .and_then(|id| this.open_tasks.iter().find(|t| t.id() == *id));
                    let id = task.map(Task::id);
                    let title = task.map_or("", Task::title).to_owned();
                    this.focus.update(cx, |f, cx| f.attach_task(id, title, cx));
                }
            }),
            cx.subscribe(&library, |this, _, event, cx| {
                if matches!(
                    event,
                    LibraryEvent::FocusChanged | LibraryEvent::TasksChanged
                ) {
                    this.reload(cx);
                }
            }),
            cx.subscribe(&focus, |this, _, event, cx| {
                if matches!(event, FocusEvent::Completed(_)) {
                    this.reload(cx);
                } else {
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &custom_input,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.commit_custom_duration(window, cx),
                    InputEvent::Blur if this.editing_custom => {
                        this.editing_custom = false;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
        ];
        let mut this = Self {
            library,
            focus,
            sessions_today: Vec::new(),
            open_tasks: Vec::new(),
            load_error: false,
            tasks_error: false,
            tasks_dirty: true,
            task_picker,
            editing_custom: false,
            custom_input,
            today: clock::today(),
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    fn open_custom_duration(
        &mut self,
        current_mins: u16,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_custom = true;
        self.custom_input.update(cx, |input, cx| {
            input.set_value(current_mins.to_string(), window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn commit_custom_duration(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.custom_input.read(cx).value().trim().to_owned();
        if let Ok(mins) = raw.parse::<u16>()
            && (MIN_DURATION_MINS..=MAX_DURATION_MINS).contains(&mins)
        {
            self.editing_custom = false;
            self.focus.update(cx, |f, cx| f.set_duration_mins(mins, cx));
        } else {
            window.push_notification(
                Notification::new().message(format!(
                    "Enter a duration from {MIN_DURATION_MINS} to {MAX_DURATION_MINS} minutes."
                )),
                cx,
            );
            return;
        }
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn cancel_custom_duration(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_custom = false;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.today = clock::today();
        let lib = self.library.read(cx);
        let (start, end) = today_bounds(self.today);
        match lib.focus_sessions_between(start, end) {
            Ok(sessions) => {
                self.sessions_today = sessions;
                self.load_error = false;
            }
            Err(err) => {
                eprintln!("ottero: couldn't load focus sessions: {err:#}");
                self.sessions_today.clear();
                self.load_error = true;
            }
        }
        match lib.tasks(TaskFilter::Open) {
            Ok(tasks) => {
                self.open_tasks = tasks;
                self.tasks_error = false;
            }
            Err(_) => {
                self.open_tasks.clear();
                self.tasks_error = true;
            }
        }
        self.tasks_dirty = true;
        cx.notify();
    }

    fn delete_session(&mut self, id: FocusSessionId, window: &mut Window, cx: &mut Context<Self>) {
        let deleted = self.sessions_today.iter().find(|s| s.id == id).cloned();
        let res = self
            .library
            .update(cx, |lib, cx| lib.delete_focus_session(id, cx));
        if let Err(err) = res {
            notify_err::<()>(Err(err), window, cx);
            return;
        }
        if let Some(session) = deleted {
            let library = self.library.clone();
            feedback::undo_toast("Session removed.", window, cx, move |window, cx| {
                let res = library.update(cx, |lib, cx| lib.record_focus_session(&session, cx));
                notify_err(res, window, cx);
            });
        }
    }

    fn mark_linked_task_done(
        &mut self,
        task_id: ottero_core::TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_status = self
            .library
            .read(cx)
            .tasks_by_id(task_id)
            .map(|task| task.status());
        let res = self.library.update(cx, |lib, cx| {
            lib.set_task_status(task_id, TaskStatus::Done, cx)
        });
        match res {
            Ok(_) => {
                self.focus.update(cx, |f, cx| f.dismiss_finished(cx));
                feedback::undo_toast("Task completed.", window, cx, {
                    let library = self.library.clone();
                    move |window, cx| {
                        let res = library.update(cx, |lib, cx| {
                            // Completed tasks must reopen before work can resume.
                            lib.set_task_status(task_id, TaskStatus::Todo, cx)?;
                            if previous_status == Some(TaskStatus::InProgress) {
                                lib.set_task_status(task_id, TaskStatus::InProgress, cx)?;
                            }
                            Ok(())
                        });
                        notify_err(res, window, cx);
                    }
                });
            }
            Err(err) => notify_err::<()>(Err(err), window, cx),
        }
    }
}

impl Focusable for FocusView {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FocusView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tasks_changed = self.tasks_dirty;
        if tasks_changed {
            self.tasks_dirty = false;
            let items: Vec<_> = self
                .open_tasks
                .iter()
                .map(|task| FocusTaskItem {
                    id: task.id(),
                    title: task.title().to_owned().into(),
                })
                .collect();
            self.task_picker.update(cx, |p, cx| {
                p.set_items(SearchableVec::new(items), window, cx)
            });
        }
        let linked = self
            .focus
            .read(cx)
            .timer()
            .task_id
            .filter(|id| self.open_tasks.iter().any(|t| t.id() == *id));
        if tasks_changed || self.task_picker.read(cx).selected_value() != linked {
            let ids: Vec<_> = linked.into_iter().collect();
            self.task_picker
                .update(cx, |p, cx| p.set_selected_values(&ids, window, cx));
        }
        let picker_focus = self.task_picker.read(cx).focus_handle(cx);
        let theme = cx.theme();
        let now = clock::now();

        let (
            timer_kind,
            planned_secs,
            remaining_secs,
            progress,
            is_running,
            is_paused,
            is_idle,
            task_id,
            task_label,
            cycle_completed,
            cycles_before_long,
            just_finished,
            save_error,
            preset,
        ) = {
            let ctrl = self.focus.read(cx);
            let t = ctrl.timer();
            (
                t.kind,
                t.planned_secs,
                t.remaining_secs(now),
                t.progress(now),
                t.is_running(),
                t.is_paused(),
                t.is_idle(),
                t.task_id,
                t.label.clone(),
                t.cycle_completed,
                t.cycles_before_long,
                ctrl.just_finished().cloned(),
                ctrl.save_error().map(str::to_owned),
                ctrl.preset(),
            )
        };

        let focus_secs_today: u64 = self
            .sessions_today
            .iter()
            .filter(|s| s.kind == FocusKind::Focus)
            .map(|s| u64::from(s.actual_secs))
            .sum();
        let focus_count_today = self
            .sessions_today
            .iter()
            .filter(|s| s.kind == FocusKind::Focus)
            .count();

        let subtitle: SharedString = if focus_count_today == 0 {
            "Quiet space for deep work".into()
        } else {
            let word = if focus_count_today == 1 {
                "session"
            } else {
                "sessions"
            };
            format!(
                "Today · {} ({focus_count_today} {word})",
                format_duration(focus_secs_today)
            )
            .into()
        };

        let ring_pct = if is_idle {
            100.0
        } else {
            ((1.0 - progress) * 100.0).clamp(0.0, 100.0)
        };
        let ring_color = if timer_kind.is_break() {
            theme.success
        } else {
            theme.primary
        };

        let planned_mins = (planned_secs / 60) as u16;
        let focus_ctrl = self.focus.clone();

        let linked_task_still_open = task_id
            .and_then(|id| self.open_tasks.iter().find(|t| t.id() == id))
            .is_some();

        v_flex()
            .id("focus-screen")
            .key_context(FOCUS_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .items_center()
            .child(
                v_flex()
                    .w_full()
                    .max_w(rems(46.))
                    .p_6()
                    .gap_6()
                    // Top header
                    .child(
                        h_flex()
                            .w_full()
                            .flex_wrap()
                            .justify_between()
                            .items_center()
                            .gap_3()
                            .child(page_header("Focus", Some(subtitle), cx))
                            .child(
                                v_flex().id("focus-mode-control").test_support().items_end()
                                    .when(!is_idle, |control| control.opacity(0.7).tooltip(|window, cx| {
                                        Tooltip::new("Finish or reset to change modes.").build(window, cx)
                                    }))
                                    .child(TabBar::new("focus-kind-tabs")
                                        .segmented().small()
                                        .selected_index(FocusKind::ALL.iter().position(|kind| *kind == timer_kind).unwrap_or(0))
                                        .children(FocusKind::ALL.into_iter().map(|kind| {
                                            Tab::new().label(kind.label())
                                        }))
                                        .on_click({
                                            let focus_ctrl = focus_ctrl.clone();
                                            move |ix, _, cx| {
                                                if let Some(kind) = FocusKind::ALL.get(*ix) {
                                                    focus_ctrl.update(cx, |f, cx| f.set_kind(*kind, cx));
                                                }
                                            }
                                        }))

                            ),
                    )
                    // Save error banner (if auto-finish failed to persist)
                    .when_some(save_error, |this, msg| {
                        this.child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .items_center()
                                .gap_3()
                                .px_4()
                                .py_3()
                                .rounded(theme.radius)
                                .border_1()
                                .border_color(theme.danger)
                                .bg(theme.group_box)
                                .child(div().text_sm().text_color(theme.danger).child(msg))
                                .child(
                                    Button::new("focus-retry-save")
                                        .outline()
                                        .small()
                                        .label("Retry save")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            let res = this
                                                .focus
                                                .update(cx, |f, cx| f.finish_now(true, cx));
                                            notify_err(res, window, cx);
                                        })),
                                ),
                        )
                    })
                    // Gentle completion banner
                    .when_some(just_finished, |this, finished| {
                        let next_kind = self.focus.read(cx).timer().next_kind_after(finished.kind);
                        let next_label = match next_kind {
                            FocusKind::Focus => "Start focus".to_owned(),
                            FocusKind::ShortBreak => {
                                format!("Start {}m break", preset.short_break_mins)
                            }
                            FocusKind::LongBreak => {
                                format!("Start {}m long break", preset.long_break_mins)
                            }
                        };
                        let finished_task_id = finished.task_id.filter(|id| {
                            finished.kind == FocusKind::Focus
                                && self.open_tasks.iter().any(|t| t.id() == *id)
                        });
                        this.child(
                            h_flex()
                                .w_full()
                                .flex_wrap()
                                .justify_between()
                                .items_center()
                                .gap_3()
                                .px_4()
                                .py_3()
                                .rounded(theme.radius_lg)
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.group_box)
                                .when(theme.shadow, |s| s.shadow_xs())
                                .child(
                                    v_flex()
                                        .gap_0p5()
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(format!("{} complete", finished.kind.label())),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child(format!(
                                                    "{} · {}",
                                                    finished.display_label(),
                                                    format_duration(u64::from(finished.actual_secs))
                                                )),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            Button::new("focus-next-block")
                                                .primary()
                                                .small()
                                                .label(next_label)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.focus.update(cx, |f, cx| {
                                                        f.start_suggested_next(window, cx);
                                                    });
                                                })),
                                        )
                                        .when_some(
                                            finished_task_id,
                                            |row, tid| {
                                                row.child(
                                                    Button::new("focus-complete-task")
                                                        .outline()
                                                        .small()
                                                        .label("Mark task done")
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                this.mark_linked_task_done(
                                                                    tid, window, cx,
                                                                );
                                                            },
                                                        )),
                                                )
                                            },
                                        )
                                        .child(
                                            Button::new("focus-dismiss-banner")
                                                .ghost()
                                                .xsmall()
                                                .icon(Lucide::X)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.focus.update(cx, |f, cx| {
                                                        f.dismiss_finished(cx);
                                                    });
                                                })),
                                        ),
                                ),
                        )
                    })
                    // Center countdown hero
                    .child(
                        v_flex()
                            .w_full()
                            .items_center()
                            .gap_5()
                            .p_6()
                            .rounded(theme.radius_lg)
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.group_box)
                            .when(theme.shadow, |s| s.shadow_xs())
                            .child(
                                ProgressCircle::new("focus-progress-ring")
                                    .value(ring_pct)
                                    .color(ring_color)
                                    .accessibility_label(format!(
                                        "{} {}",
                                        timer_kind.label(),
                                        format_mm_ss(remaining_secs)
                                    ))
                                    .size(rems(16.))
                                    .child(
                                        v_flex()
                                            .items_center()
                                            .gap_1p5()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme.muted_foreground)
                                                    .child(timer_kind.label()),
                                            )
                                            .child(
                                                div()
                                                    .id("focus-clock")
                                                    .text_3xl()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .when(is_idle, |el| {
                                                        el.cursor_pointer().on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                this.open_custom_duration(
                                                                    planned_mins,
                                                                    window,
                                                                    cx,
                                                                );
                                                            },
                                                        ))
                                                    })
                                                    .child(format_mm_ss(remaining_secs)),
                                            )
                                            .child(
                                                h_flex()
                                                    .gap_1p5()
                                                    .pt_1()
                                                    .children((0..cycles_before_long).map(|ix| {
                                                        let filled = ix < cycle_completed;
                                                        let current =
                                                            ix == cycle_completed
                                                                && timer_kind == FocusKind::Focus
                                                                && (is_running || is_paused);
                                                        let dot_color = if filled {
                                                            theme.primary
                                                        } else if current {
                                                            theme.primary.opacity(0.4)
                                                        } else {
                                                            theme.border
                                                        };
                                                        div()
                                                            .size(rems(0.375))
                                                            .rounded_full()
                                                            .bg(dot_color)
                                                    })),
                                            ),
                                    ),
                            )
                            // One quiet duration control instead of an always-visible preset row.
                            .when(is_idle, |col| {
                                col.child(if self.editing_custom {
                                    v_flex().items_center().gap_1()
                                        .child(div().w(rems(6.)).on_key_down(cx.listener(
                                            |this, event: &KeyDownEvent, window, cx| {
                                                if event.keystroke.key == "escape" {
                                                    this.cancel_custom_duration(window, cx);
                                                }
                                            },
                                        )).child(Input::new(&self.custom_input)
                                            .id("focus-custom-input").small()
                                            .suffix(div().text_xs().child("min"))))
                                        .child(div().text_xs().text_color(theme.muted_foreground).child(format!("{MIN_DURATION_MINS}–{MAX_DURATION_MINS} minutes · Enter to apply")))
                                        .into_any_element()
                                } else {
                                    let view = cx.entity();
                                    Button::new("focus-duration")
                                        .ghost().small().icon(Lucide::ChevronDown)
                                        .label(format!("{planned_mins} minutes"))
                                        .tooltip("Change duration")
                                        .dropdown_menu(move |mut menu, _, _| {
                                            let presets: Vec<u16> = match timer_kind {
                                                FocusKind::Focus => FOCUS_DURATION_PRESETS.to_vec(),
                                                FocusKind::ShortBreak => vec![3, 5, 10],
                                                FocusKind::LongBreak => vec![15, 20, 30],
                                            };
                                            for mins in presets {
                                                let view = view.clone();
                                                menu = menu.item(gpui_kit::component::menu::PopupMenuItem::new(format!("{mins} minutes"))
                                                    .checked(planned_mins == mins)
                                                    .on_click(move |_, _, cx| view.update(cx, |this, cx| {
                                                        this.focus.update(cx, |f, cx| f.set_duration_mins(mins, cx));
                                                    })));
                                            }
                                            let view = view.clone();
                                            menu.separator().item(gpui_kit::component::menu::PopupMenuItem::new("Custom duration…")
                                                .on_click(move |_, window, cx| view.update(cx, |this, cx| {
                                                    this.open_custom_duration(planned_mins, window, cx);
                                                })))
                                        }).into_any_element()
                                })
                            })
                            // Linked task selector
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        Combobox::new(&self.task_picker)
                                            .render_trigger(move |trigger, window, cx| {
                                                let label = trigger.selection().first()
                                                    .map_or_else(|| trigger.placeholder().cloned().unwrap_or_else(|| SharedString::from("Link a task")), |(_, item)| item.title.clone());
                                                h_flex().id("focus-task-picker").test_support()
                                                    .gap_2().w_full().items_center().border_1().border_color(cx.theme().transparent).rounded(cx.theme().radius)
                                                    .when(picker_focus.is_focused(window) || trigger.is_open(), |row| row.border_color(cx.theme().ring))
                                                    .child(div().flex_1().min_w_0().truncate().child(label))
                                                    .child(gpui_kit::component::Icon::new(Lucide::ChevronDown).xsmall()
                                                        .text_color(cx.theme().muted_foreground))
                                            })
                                            .appearance(false)
                                            .small()
                                            .w(rems(16.))
                                            .menu_width(rems(22.))
                                            .menu_max_h(rems(18.))
                                            .placeholder(if task_label.is_empty() { "Link a task".to_owned() } else { task_label.clone() })
                                            .search_placeholder("Search all open tasks…")
                                            .empty({
                                                let failed = self.tasks_error;
                                                let no_tasks = self.open_tasks.is_empty();
                                                move |_, cx| div().p_4().text_sm().text_color(cx.theme().muted_foreground)
                                                    .child(if failed { "Couldn't load tasks. Reopen Focus to retry." }
                                                        else if no_tasks { "No open tasks" }
                                                        else { "No matching tasks" })
                                            })
                                    )
                                    .when_some(
                                        task_id.filter(|_| linked_task_still_open),
                                        |row, tid| {
                                            row.child(
                                                Button::new("focus-open-linked-task")
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(Lucide::ExternalLink)
                                                    .tooltip("Open task")
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            open_task_detail(
                                                                tid,
                                                                this.library.clone(),
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                            )
                                        },
                                    )
                                    .when(task_id.is_some() || !task_label.is_empty(), |row| {
                                        row.child(
                                            Button::new("focus-clear-task")
                                                .ghost()
                                                .xsmall()
                                                .icon(Lucide::X)
                                                .tooltip("Unlink task")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.focus.update(cx, |f, cx| {
                                                        f.attach_task(None, "", cx);
                                                    });
                                                })),
                                        )
                                    }),
                            )
                            // Primary controls
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(if is_idle {
                                        Button::new("focus-toggle")
                                            .ghost()
                                            .icon(Lucide::Play)
                                            .tooltip("Start timer (Space)")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.focus.update(cx, |f, cx| f.toggle(window, cx));
                                            }))
                                    } else {
                                        Button::new("focus-toggle")
                                            .ghost()
                                            .icon(if is_running {
                                                Lucide::Pause
                                            } else {
                                                Lucide::Play
                                            })
                                            .tooltip(if is_running {
                                                "Pause (Space)"
                                            } else {
                                                "Resume (Space)"
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.focus.update(cx, |f, cx| f.toggle(window, cx));
                                            }))
                                    })
                                    .when(!is_idle, |row| {
                                        row.child(
                                            Button::new("focus-finish")
                                                .ghost()
                                                .icon(Lucide::CircleCheckBig)
                                                .tooltip("Finish & log session")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    let res = this.focus.update(cx, |f, cx| {
                                                        f.finish_now(true, cx)
                                                    });
                                                    notify_err(res, window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("focus-reset")
                                                .ghost()
                                                .icon(Lucide::RotateCcw)
                                                .tooltip("Reset (R)")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.focus.update(cx, |f, cx| f.reset(cx));
                                                })),
                                        )
                                    }),
                            )

                    )
                    // Today's log
                    .child(
                        v_flex()
                            .w_full()
                            .gap_2()
                            .child(section_label("Today's log", cx))
                            .child(if self.load_error {
                                div()
                                    .rounded(theme.radius_lg)
                                    .border_1()
                                    .border_color(theme.border)
                                    .bg(theme.group_box)
                                    .when(theme.shadow, |s| s.shadow_xs())
                                    .child(empty_state(
                                        Lucide::TriangleAlert,
                                        "Couldn't load today's sessions",
                                        "Something went wrong reading your focus log.",
                                        cx,
                                    ))
                                    .into_any_element()
                            } else if self.sessions_today.is_empty() {
                                div()
                                    .rounded(theme.radius_lg)
                                    .border_1()
                                    .border_color(theme.border)
                                    .bg(theme.group_box)
                                    .when(theme.shadow, |s| s.shadow_xs())
                                    .child(empty_state(
                                        Lucide::Timer,
                                        "No sessions today",
                                        "Pick a task or press Space to start a calm focus block.",
                                        cx,
                                    ))
                                    .into_any_element()
                            } else {
                                v_flex()
                                    .w_full()
                                    .rounded(theme.radius_lg)
                                    .border_1()
                                    .border_color(theme.border)
                                    .bg(theme.group_box)
                                    .when(theme.shadow, |s| s.shadow_xs())
                                    .children(self.sessions_today.iter().enumerate().map(
                                        |(ix, session)| {
                                            let sid = session.id;
                                            let time_span = format!(
                                                "{} – {}",
                                                format_local_time(session.started_at),
                                                format_local_time(session.ended_at),
                                            );
                                            let dur =
                                                format_duration(u64::from(session.actual_secs));
                                            h_flex()
                                                .id(("focus-log-row", ix))
                                                .w_full()
                                                .items_center()
                                                .justify_between()
                                                .gap_3()
                                                .px_3()
                                                .py_2()
                                                .when(ix > 0, |row| {
                                                    row.border_t_1().border_color(theme.border)
                                                })
                                                .child(
                                                    h_flex()
                                                        .items_center()
                                                        .gap_3()
                                                        .min_w_0()
                                                        .child(
                                                            div()
                                                                .flex_shrink_0()
                                                                .text_xs()
                                                                .text_color(theme.muted_foreground)
                                                                .child(time_span),
                                                        )
                                                        .when(session.kind.is_break(), |row| {
                                                            row.child(
                                                                div()
                                                                    .flex_shrink_0()
                                                                    .text_xs()
                                                                    .text_color(
                                                                        theme.muted_foreground,
                                                                    )
                                                                    .child(session.kind.label()),
                                                            )
                                                        })
                                                        .child(
                                                            div()
                                                                .min_w_0()
                                                                .text_sm()
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .truncate()
                                                                .child(
                                                                    session
                                                                        .display_label()
                                                                        .to_owned(),
                                                                ),
                                                        ),
                                                )
                                                .child(
                                                    h_flex()
                                                        .items_center()
                                                        .gap_2()
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(theme.muted_foreground)
                                                                .child(dur),
                                                        )
                                                        .child(
                                                            Button::new((
                                                                "focus-log-del",
                                                                ix,
                                                            ))
                                                            .ghost()
                                                            .xsmall()
                                                            .icon(Lucide::Trash)
                                                            .tooltip("Remove session")
                                                            .on_click(cx.listener(
                                                                move |this, _, window, cx| {
                                                                    this.delete_session(
                                                                        sid, window, cx,
                                                                    );
                                                                },
                                                            )),
                                                        ),
                                                )
                                        },
                                    ))
                                    .into_any_element()
                            }),
                    ),
            )
    }
}

fn today_bounds(today: Date) -> (ottero_core::Timestamp, ottero_core::Timestamp) {
    let tz = jiff::tz::TimeZone::system();
    let start = today
        .at(0, 0, 0, 0)
        .to_zoned(tz.clone())
        .map_or(ottero_core::Timestamp::UNIX_EPOCH, |z| z.timestamp());
    let end = today
        .tomorrow()
        .ok()
        .and_then(|d| d.at(0, 0, 0, 0).to_zoned(tz).ok())
        .map_or(start, |z| z.timestamp());
    (start, end)
}

fn format_local_time(ts: ottero_core::Timestamp) -> String {
    ts.to_zoned(jiff::tz::TimeZone::system())
        .strftime("%H:%M")
        .to_string()
}
