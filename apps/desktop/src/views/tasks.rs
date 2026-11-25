//! Tasks screen: quick add plus Today / Upcoming / All / Done / Trash lists.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, rems,
};
use std::collections::HashMap;

use ottero_core::{Date, ProjectId, Task, TaskId, clock};
use ottero_storage::{Progress, TaskFilter};
use ottero_ui::components::{empty_state, page_header, segmented};
use ottero_ui::feedback::{self, Confirm};

use crate::library::{Library, LibraryEvent};
use crate::views::task_row::{notify_err, task_row};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskTab {
    Today,
    Upcoming,
    All,
    Done,
    Trash,
}

impl TaskTab {
    const OPTIONS: [(Self, &'static str); 5] = [
        (Self::Today, "Today"),
        (Self::Upcoming, "Upcoming"),
        (Self::All, "All"),
        (Self::Done, "Done"),
        (Self::Trash, "Trash"),
    ];

    fn filter(self, today: Date) -> TaskFilter {
        match self {
            Self::Today => TaskFilter::Today(today),
            Self::Upcoming => TaskFilter::Upcoming(today),
            Self::All => TaskFilter::Open,
            Self::Done => TaskFilter::Closed,
            Self::Trash => TaskFilter::Trash,
        }
    }
}

pub struct TasksView {
    library: Entity<Library>,
    input: Entity<InputState>,
    tab: TaskTab,
    /// When set, the screen shows this project's open tasks instead of a tab.
    project: Option<ProjectId>,
    /// What quick add understood from the current input ("Tomorrow · High · Repeats daily").
    hint: Option<SharedString>,
    progress: HashMap<TaskId, Progress>,
    tasks: Vec<Task>,
    load_error: bool,
    today: Date,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl TasksView {
    pub fn new(library: Entity<Library>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Add a task and press Enter"));
        let subscriptions = vec![
            cx.subscribe(&library, |this, _, event, cx| {
                if *event == LibraryEvent::TasksChanged {
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
                        this.hint = quick_add_hint(&text, this.today, &this.library, cx);
                        cx.notify();
                    }
                    _ => {}
                },
            ),
        ];
        let mut this = Self {
            library,
            input,
            tab: TaskTab::Today,
            project: None,
            hint: None,
            progress: HashMap::new(),
            tasks: Vec::new(),
            load_error: false,
            today: clock::today(),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    /// Focuses the quick-add field (used by the "New task" action).
    pub fn focus_new_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab == TaskTab::Done || self.tab == TaskTab::Trash {
            self.set_tab(TaskTab::Today, cx);
        }
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn tab(&self) -> TaskTab {
        self.tab
    }

    pub fn set_tab(&mut self, tab: TaskTab, cx: &mut Context<Self>) {
        if self.tab != tab || self.project.is_some() {
            self.tab = tab;
            self.project = None;
            self.reload(cx);
        }
    }

    pub fn project(&self) -> Option<ProjectId> {
        self.project
    }

    /// Shows one project's open tasks. New tasks added here join the project.
    pub fn show_project(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        self.project = Some(id);
        self.reload(cx);
    }

    fn filter(&self) -> TaskFilter {
        match self.project {
            Some(id) => TaskFilter::Project(id),
            None => self.tab.filter(self.today),
        }
    }

    /// Reloads when the date changed since the last load (app left open overnight).
    pub fn refresh_if_new_day(&mut self, cx: &mut Context<Self>) {
        if clock::today() != self.today {
            self.reload(cx);
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.today = clock::today();
        let library = self.library.read(cx);
        // A deleted project falls back to the tab view.
        if self.project.is_some_and(|id| library.project(id).is_none()) {
            self.project = None;
        }
        self.progress = library.subtask_progress();
        match library.tasks(self.filter()) {
            Ok(tasks) => {
                self.tasks = tasks;
                self.load_error = false;
            }
            Err(err) => {
                eprintln!("ottero: failed to load tasks: {err:#}");
                self.tasks.clear();
                self.load_error = true;
            }
        }
        cx.notify();
    }

    fn add_from_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.input.read(cx).value().to_string();
        if title.trim().is_empty() {
            return;
        }
        // New tasks land where the user is looking: Today → due today,
        // Upcoming → due tomorrow; a date typed in the text wins.
        let due = match (self.project, self.tab) {
            (Some(_), _) => None,
            (None, TaskTab::Today) => Some(self.today),
            (None, TaskTab::Upcoming) => self.today.tomorrow().ok(),
            _ => None,
        };
        let project = self.project;
        let result = self
            .library
            .update(cx, |lib, cx| lib.quick_add_task(&title, due, project, cx));
        if result.is_ok() {
            self.input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.hint = None;
        }
        notify_err(result, window, cx);
    }
}

impl TasksView {
    fn empty_trash(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let library = self.library.clone();
        feedback::confirm_destructive(
            Confirm::new(
                "Empty task trash?",
                format!(
                    "{} task(s) will be deleted forever. This can't be undone.",
                    self.tasks.len()
                ),
                "Empty trash",
            ),
            window,
            cx,
            move |window, cx| {
                notify_err(
                    library.update(cx, |lib, cx| lib.empty_task_trash(cx)),
                    window,
                    cx,
                );
            },
        );
    }
}

impl TasksView {
    fn rename_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self
            .project
            .and_then(|id| self.library.read(cx).project(id))
        else {
            return;
        };
        let library = self.library.clone();
        let id = project.id();
        feedback::prompt_text(
            "Rename project",
            "Project name",
            project.name().to_owned(),
            "Rename",
            window,
            cx,
            move |name, window, cx| {
                notify_err(
                    library.update(cx, |lib, cx| lib.rename_project(id, &name, cx)),
                    window,
                    cx,
                );
            },
        );
    }

    fn delete_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self
            .project
            .and_then(|id| self.library.read(cx).project(id))
        else {
            return;
        };
        let library = self.library.clone();
        let id = project.id();
        feedback::confirm_destructive(
            Confirm::new(
                format!("Delete \u{201c}{}\u{201d}?", project.name()),
                "Its tasks are kept and move out of the project.",
                "Delete project",
            ),
            window,
            cx,
            move |window, cx| {
                notify_err(
                    library.update(cx, |lib, cx| lib.delete_project(id, cx)),
                    window,
                    cx,
                )
            },
        );
    }
}

/// Human summary of what quick add will set, or `None` for a plain title.
fn quick_add_hint(
    text: &str,
    today: Date,
    library: &Entity<Library>,
    cx: &gpui_kit::App,
) -> Option<SharedString> {
    let parsed = ottero_core::parse_quick_add(text, today);
    if !parsed.has_extras() {
        return None;
    }
    let mut parts = Vec::new();
    if let Some(due) = parsed.due {
        parts.push(crate::views::relative_day(due, today));
    }
    if let Some(p) = parsed.priority {
        parts.push(format!("{} priority", p.label()));
    }
    if let Some(r) = parsed.recurrence {
        parts.push(r.to_string());
    }
    if let Some(name) = parsed.project {
        let exists = library
            .read(cx)
            .projects()
            .iter()
            .any(|(p, _)| p.name().eq_ignore_ascii_case(&name));
        parts.push(if exists {
            format!("in {name}")
        } else {
            format!("#{name} (no such project, kept in title)")
        });
    }
    Some(format!("{} — {}", parsed.title, parts.join(" · ")).into())
}

impl Focusable for TasksView {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TasksView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let tab_bar = segmented(
            "task-tabs",
            &TaskTab::OPTIONS,
            self.tab,
            move |tab, _, cx| {
                _ = view.update(cx, |this, cx| this.set_tab(tab, cx));
            },
        );

        let project = self
            .project
            .and_then(|id| self.library.read(cx).project(id));
        let (empty_title, empty_desc) = match self.tab {
            _ if project.is_some() && !self.load_error => (
                "No open tasks in this project",
                "Add one above. Tip: type #Project in any quick add to file it here.",
            ),
            _ if self.load_error => (
                "Couldn't load tasks",
                "Your tasks are still on disk. Restart Ottero; if this keeps happening, check the data folder in Settings.",
            ),
            TaskTab::Today => (
                "Nothing due today",
                "Add a task above. Tasks added here are due today.",
            ),
            TaskTab::Upcoming => ("Nothing upcoming", "Give a task a due date to see it here."),
            TaskTab::All => ("No open tasks", "Add a task above to get started."),
            TaskTab::Done => ("Nothing finished yet", "Completed tasks show up here."),
            TaskTab::Trash => (
                "Trash is empty",
                "Tasks you delete stay here until you empty the trash.",
            ),
        };
        let show_input = project.is_some() || !matches!(self.tab, TaskTab::Done | TaskTab::Trash);
        let title: SharedString = project
            .as_ref()
            .map_or("Tasks".into(), |p| p.name().to_owned().into());
        let view = cx.entity().downgrade();
        let project_menu = project.is_some().then(|| {
            Button::new("project-more")
                .ghost()
                .small()
                .icon(Lucide::Ellipsis)
                .tooltip("Project")
                .dropdown_menu(move |menu, _, _| {
                    let rename = view.clone();
                    let delete = view.clone();
                    menu.item(PopupMenuItem::new("Rename…").icon(Lucide::Pencil).on_click(
                        move |_, window, cx| {
                            _ = rename.update(cx, |this, cx| this.rename_project(window, cx));
                        },
                    ))
                    .separator()
                    .item(
                        PopupMenuItem::new("Delete project…")
                            .icon(Lucide::Trash)
                            .on_click(move |_, window, cx| {
                                _ = delete.update(cx, |this, cx| this.delete_project(window, cx));
                            }),
                    )
                })
        });

        v_flex()
            .track_focus(&self.focus)
            .size_full()
            .child(
                v_flex()
                    .w_full()
                    .max_w(rems(48.))
                    .mx_auto()
                    .px_6()
                    .pt_6()
                    .gap_4()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_end()
                            .gap_4()
                            .child(h_flex().gap_1().items_center().child(page_header(title, None, cx)).children(project_menu))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .when(
                                        self.tab == TaskTab::Trash && !self.tasks.is_empty(),
                                        |this| {
                                            this.child(
                                                Button::new("tasks-empty-trash")
                                                    .ghost()
                                                    .small()
                                                    .label("Empty trash")
                                                    .on_click(cx.listener(Self::empty_trash)),
                                            )
                                        },
                                    )
                                    .when(self.project.is_none(), |this| this.child(tab_bar)),
                            ),
                    )
                    .children(show_input.then(|| {
                        v_flex()
                            .gap_1()
                            .child(Input::new(&self.input).id("task-input").prefix(
                                Icon::new(Lucide::Plus).small().text_color(cx.theme().muted_foreground),
                            ))
                            .child(
                                div()
                                    .h(rems(1.25))
                                    .px_1()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(self.hint.clone().unwrap_or_else(|| {
                                        "Try “Pay rent tomorrow !high”, “Gym every monday”, “Call bank fri #Home”".into()
                                    })),
                            )
                    })),
            )
            .child(
                div()
                    .id("task-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .w_full()
                            .max_w(rems(48.))
                            .mx_auto()
                            .px_6()
                            .py_3()
                            .gap_0p5()
                            .map(|list| {
                                if self.tasks.is_empty() {
                                    list.child(empty_state(
                                        Lucide::ListTodo,
                                        empty_title,
                                        empty_desc,
                                        cx,
                                    ))
                                } else {
                                    let theme = cx.theme();
                                    list.child(
                                        v_flex()
                                            .p_2()
                                            .gap_0p5()
                                            .bg(theme.group_box)
                                            .rounded(theme.radius_lg)
                                            .border_1()
                                            .border_color(theme.border)
                                            .when(theme.shadow, |s| s.shadow_xs())
                                            .children(self.tasks.iter().map(|task| {
                                                task_row(
                                                    task,
                                                    self.today,
                                                    self.progress.get(&task.id()).copied(),
                                                    &self.library,
                                                    cx,
                                                )
                                            })),
                                    )
                                }
                            }),
                    ),
            )
    }
}
