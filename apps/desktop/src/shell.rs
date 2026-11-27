//! The main window: title bar, sidebar, the active screen, and the command palette.
//!
//! Commands are app-level action handlers that forward to the shell. That way
//! a shortcut, a menu-bar item, and a palette entry all run the same code no
//! matter which element has focus (the palette lives in an overlay outside the
//! shell's element tree).

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::InteractiveElementExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::resizable::{HandleEdge, resize_handle};
use gpui_kit::component::sidebar::{
    Sidebar, SidebarCollapsible, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem,
};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, TitleBar, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Action, AnyView, AnyWindowHandle, App, AppContext as _, Context, Entity, FocusHandle,
    Focusable as _, Global, InteractiveElement as _, IntoElement, Keystroke, ParentElement as _,
    Render, Styled as _, Subscription, TestSupportExt as _, WeakEntity, Window, div, px, rems,
};
use ottero_core::NoteId;
use ottero_ui::appearance::{self, ThemePreference};

use crate::actions::{
    ChangeMasterPassword, CheckForUpdates, CopyProjectEnvironment, CreateRecoveryKey,
    CustomizeSidebar, ExportNotes, ExportProjectEnvironment, ExportVaultBackup, FindInNote,
    FindNext, FindPrevious, FocusFinish, FocusReset, FocusToggle, GeneratePassword, GoCalendar,
    GoDeveloper, GoFocus, GoHabits, GoNotes, GoPasswords, GoSettings, GoTasks, GoToday,
    ImportCalendar, ImportNotes, ImportProjectEnvironment, LockVault, NewDeveloperProject,
    NewEvent, NewHabit, NewLogin, NewNote, NewProject, NewProjectSecrets, NewTask,
    OpenCommandPalette, OpenDailyNote, PasteProjectEnvironment, RestartToUpdate,
    RestoreVaultBackup, ShowDeletedEvents, ShowVaultArchives, ToggleArchivedDeveloperProjects,
    ToggleListPanel, ToggleMarkdownSource, ToggleSidebar, UseAccountForPasswords, UseDarkTheme,
    UseLightTheme, UseSystemTheme,
};
use crate::focus::FocusController;
use crate::library::{Library, LibraryEvent};
use crate::nav::{self, Badge, NavTarget};
use crate::paths::AppPaths;
use crate::ui_state::{self, LayoutState, LayoutStore};
use crate::views::calendar::CalendarView;
use crate::views::developer::DeveloperView;
use crate::views::focus::FocusView;
use crate::views::habits::HabitsView;
use crate::views::notes::NotesView;
use crate::views::passwords::PasswordsView;
use crate::views::settings::SettingsView;
use crate::views::tasks::TasksView;
use crate::views::today::TodayView;
use crate::views::{Navigate, Route};

const PALETTE_NOTE_LIMIT: usize = 200;
/// Vault tags listed in the sidebar (the rest are a search away: `tag:`).
const SIDEBAR_TAGS: usize = 6;

/// Drag payload for the sidebar's resize edge.
#[derive(Clone)]
struct ResizeSidebar;

impl Render for ResizeSidebar {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::Empty
    }
}

#[derive(Clone, Copy)]
enum EnvironmentCommand {
    Import,
    Paste,
    Copy,
    Export,
}

pub struct AppShell {
    route: Route,
    layout: Entity<LayoutStore>,
    /// Set while the command palette is the open dialog.
    palette_open: std::rc::Rc<std::cell::Cell<bool>>,
    due_now: usize,
    habits_left: usize,
    library: Entity<Library>,
    today: Entity<TodayView>,
    notes: Entity<NotesView>,
    tasks: Entity<TasksView>,
    calendar: Entity<CalendarView>,
    habits: Entity<HabitsView>,
    focus_ctrl: Entity<FocusController>,
    focus_view: Entity<FocusView>,
    developer: Entity<DeveloperView>,
    passwords: Entity<PasswordsView>,
    settings: Entity<SettingsView>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// Lets app-level action handlers reach the shell of the main window.
struct ShellHandle {
    window: AnyWindowHandle,
    shell: WeakEntity<AppShell>,
}

impl Global for ShellHandle {}

impl AppShell {
    pub fn new(
        library: Entity<Library>,
        paths: &AppPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let layout = cx.new(|_| LayoutStore::load(&paths.data_dir));
        let today = cx.new(|cx| TodayView::new(library.clone(), cx));
        let notes = cx.new(|cx| NotesView::new(library.clone(), layout.clone(), window, cx));
        let tasks = cx.new(|cx| TasksView::new(library.clone(), window, cx));
        let calendar = cx.new(|cx| CalendarView::new(library.clone(), window, cx));
        let habits = cx.new(|cx| HabitsView::new(library.clone(), window, cx));
        let focus_ctrl = cx.new(|_| FocusController::new(library.clone()));
        crate::focus::install(&focus_ctrl, cx);
        let focus_view =
            cx.new(|cx| FocusView::new(library.clone(), focus_ctrl.clone(), window, cx));
        let passwords = cx.new(|cx| PasswordsView::new(library.clone(), window, cx));
        let developer = cx.new(|cx| DeveloperView::new(library.clone(), window, cx));
        let account =
            cx.new(|cx| crate::views::account::AccountView::new(library.clone(), window, cx));
        let settings =
            cx.new(|cx| SettingsView::new(paths.data_dir.clone(), layout.clone(), account, cx));

        let subscriptions = vec![
            cx.subscribe_in(&developer, window, |this, _, nav, window, cx| {
                this.handle_nav(*nav, window, cx)
            }),
            cx.observe(&layout, |_, _, cx| cx.notify()),
            cx.observe(&focus_ctrl, |_, _, cx| cx.notify()),
            cx.subscribe_in(&today, window, |this, _, nav, window, cx| {
                this.handle_nav(*nav, window, cx)
            }),
            cx.subscribe_in(
                &today,
                window,
                |this, _, _: &crate::views::today::OpenDailyNoteRequest, window, cx| {
                    this.open_daily_note(window, cx)
                },
            ),
            cx.subscribe_in(&notes, window, |this, _, nav, window, cx| {
                this.handle_nav(*nav, window, cx)
            }),
            cx.subscribe(&library, |this, _, event, cx| {
                if matches!(
                    event,
                    LibraryEvent::TasksChanged
                        | LibraryEvent::HabitsChanged
                        | LibraryEvent::VaultChanged
                ) {
                    this.refresh_badges(cx);
                }
                if matches!(event, LibraryEvent::AccountChanged) {
                    cx.notify();
                } else {
                    let immediate = !matches!(event, LibraryEvent::NotesChanged);
                    this.library.update(cx, |lib, cx| {
                        if lib.can_auto_sync() && lib.pending_sync_count() > 0 {
                            if immediate {
                                lib.schedule_immediate_sync(cx);
                            } else {
                                lib.schedule_debounced_sync(cx);
                            }
                        }
                    });
                }
            }),
            // ⌘Q or closing the window must not drop typing that is still
            // waiting for the autosave timer, or an unsaved sidebar width.
            cx.on_app_quit(|this, cx| {
                this.save_everything(cx);
                async {}
            }),
            cx.observe_window_appearance(window, |_, window, cx| {
                appearance::system_appearance_changed(window, cx);
            }),
            // Refresh "today" when the window comes back to the front (date may have changed).
            // Any key counts as activity for vault auto-lock.
            cx.observe_keystrokes(|this, _, _, cx| {
                this.library.update(cx, |lib, _| lib.touch_vault());
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() && crate::vault_prefs::get(cx).lock_on_hide {
                    this.library.update(cx, |lib, cx| lib.lock_vault(cx));
                }
                if window.is_window_active() {
                    this.today.update(cx, |today, cx| today.reload(cx));
                    this.tasks
                        .update(cx, |tasks, cx| tasks.refresh_if_new_day(cx));
                    this.calendar.update(cx, |calendar, cx| calendar.reload(cx));
                    this.habits.update(cx, |habits, cx| habits.reload(cx));
                    this.focus_view.update(cx, |focus, cx| focus.reload(cx));
                    this.refresh_badges(cx);
                    this.library.update(cx, |lib, cx| lib.sync_on_focus(cx));
                }
            }),
        ];

        let startup = layout.read(cx).navigation().startup_route();
        let mut this = Self {
            route: Route::Today,
            layout,
            palette_open: std::rc::Rc::default(),
            due_now: 0,
            habits_left: 0,
            library,
            today,
            notes,
            tasks,
            calendar,
            habits,
            focus_ctrl,
            focus_view,
            passwords,
            developer,
            settings,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.refresh_badges(cx);
        this.navigate(startup, window, cx);
        this
    }

    /// Flushes everything that is saved lazily.
    pub fn save_everything(&mut self, cx: &mut Context<Self>) {
        self.layout.update(cx, |layout, _| layout.save_now());
        if let Err(err) = self.notes.update(cx, |notes, cx| notes.save_pending(cx)) {
            eprintln!("ottero: could not save the open note on exit: {err:#}");
        }
    }

    /// Updating is optional: a failed save must leave the running app and its
    /// drafts intact. Unlike the OS quit callback, this path can refuse exit.
    pub fn prepare_update_restart(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.focus_ctrl.read(cx).timer().is_idle() {
            ottero_ui::feedback::error_toast(
                "Finish or reset your timer before restarting to update.",
                window,
                cx,
            );
            return false;
        }
        // Sheets own their own autosave/error state (including task details).
        // Do not close one implicitly or quit before its draft can be reviewed.
        if window.has_active_sheet(cx) || window.has_active_dialog(cx) {
            ottero_ui::feedback::error_toast(
                "Close the open editor or dialog before restarting to update.",
                window,
                cx,
            );
            return false;
        }
        if self
            .notes
            .update(cx, |notes, cx| notes.save_pending(cx))
            .is_err()
        {
            ottero_ui::feedback::error_toast(
                "Your note couldn't be saved. The update was not installed.",
                window,
                cx,
            );
            return false;
        }
        if !self.developer.update(cx, |view, cx| view.flush(window, cx))
            || !self
                .passwords
                .update(cx, |view, cx| view.flush_edit(window, cx))
        {
            return false;
        }
        self.layout.update(cx, |layout, _| layout.save_now());
        true
    }

    fn refresh_badges(&mut self, cx: &mut Context<Self>) {
        let counts = self.library.read(cx).task_counts().unwrap_or_default();
        self.due_now = counts.due_today + counts.overdue;
        self.habits_left = self
            .library
            .read(cx)
            .habits(false)
            .map(|hs| hs.iter().filter(|h| h.due_today && !h.done_today()).count())
            .unwrap_or(0);
        cx.notify();
    }

    fn set_sidebar_width(&mut self, width: f32, cx: &mut Context<Self>) {
        self.layout
            .update(cx, |layout, cx| layout.set_sidebar_width(width, cx));
    }

    /// Opens a sidebar deep link.
    pub fn open_target(&mut self, target: NavTarget, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            NavTarget::Route(route) => self.navigate(route, window, cx),
            NavTarget::Tasks(tab) => {
                self.navigate(Route::Tasks, window, cx);
                self.tasks.update(cx, |tasks, cx| tasks.set_tab(tab, cx));
            }
            NavTarget::NotesAll | NavTarget::NotesTrash => {
                self.navigate(Route::Notes, window, cx);
                let trash = target == NavTarget::NotesTrash;
                self.notes
                    .update(cx, |notes, cx| notes.set_show_trash(trash, window, cx));
            }
            NavTarget::DailyNote => self.open_daily_note(window, cx),
            NavTarget::Project(id) => {
                self.navigate(Route::Tasks, window, cx);
                self.tasks
                    .update(cx, |tasks, cx| tasks.show_project(id, cx));
            }
            NavTarget::Vault(filter) => {
                self.navigate(Route::Passwords, window, cx);
                self.passwords
                    .update(cx, |p, cx| p.set_filter(filter, window, cx));
            }
        }
    }

    fn is_target_active(&self, target: NavTarget, cx: &App) -> bool {
        match target {
            NavTarget::Route(route) => self.route == route,
            NavTarget::Tasks(tab) => {
                self.route == Route::Tasks
                    && self.tasks.read(cx).project().is_none()
                    && self.tasks.read(cx).tab() == tab
            }
            NavTarget::Project(id) => {
                self.route == Route::Tasks && self.tasks.read(cx).project() == Some(id)
            }
            NavTarget::Vault(filter) => {
                self.route == Route::Passwords
                    && self.passwords.read(cx).filter() == filter
                    && self.passwords.read(cx).tag().is_none()
            }
            NavTarget::DailyNote => {
                self.route == Route::Notes
                    && self.notes.read(cx).open_note_title(cx).as_deref()
                        == Some(
                            crate::library::daily_note_title(ottero_core::clock::today()).as_str(),
                        )
            }
            NavTarget::NotesAll => {
                self.route == Route::Notes && !self.notes.read(cx).is_showing_trash()
            }
            NavTarget::NotesTrash => {
                self.route == Route::Notes && self.notes.read(cx).is_showing_trash()
            }
        }
    }

    fn badge_count(&self, badge: Badge) -> usize {
        match badge {
            Badge::None => 0,
            Badge::TasksDueNow => self.due_now,
            Badge::HabitsLeft => self.habits_left,
        }
    }

    /// Registers this shell as the target of app-level commands.
    pub fn install(shell: &Entity<Self>, window: AnyWindowHandle, cx: &mut App) {
        cx.set_global(ShellHandle {
            window,
            shell: shell.downgrade(),
        });
        // One quiet notice when an update is ready or a manual check ends.
        let mut last = crate::updates::Status::Idle;
        cx.observe_global::<crate::updates::Updates>(move |cx| {
            let status = crate::updates::status(cx);
            if status == last {
                return;
            }
            last = status.clone();
            _ = window.update(cx, |_, window, cx| update_notice(&status, window, cx));
        })
        .detach();
    }

    pub fn navigate(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        if self.route == Route::Developer
            && route != Route::Developer
            && !self.developer.update(cx, |d, cx| d.flush(window, cx))
        {
            return;
        }
        self.route = route;
        match route {
            Route::Today => self.today.update(cx, |today, cx| today.reload(cx)),
            Route::Notes => self
                .notes
                .update(cx, |notes, cx| notes.ensure_selection(window, cx)),
            Route::Calendar => self.calendar.update(cx, |c, cx| c.reload(cx)),
            Route::Habits => self.habits.update(cx, |h, cx| h.reload(cx)),
            Route::Focus => self.focus_view.update(cx, |f, cx| f.reload(cx)),
            Route::Developer => self.developer.update(cx, |d, cx| d.reload(window, cx)),
            Route::Passwords => {}
            Route::Tasks | Route::Settings => {}
        }
        self.focus_route(window, cx);
        cx.notify();
    }

    fn focus_route(&self, window: &mut Window, cx: &mut App) {
        let handle = match self.route {
            Route::Today => self.today.focus_handle(cx),
            Route::Notes => self.notes.focus_handle(cx),
            Route::Tasks => self.tasks.focus_handle(cx),
            Route::Calendar => self.calendar.focus_handle(cx),
            Route::Habits => self.habits.focus_handle(cx),
            Route::Focus => self.focus_view.focus_handle(cx),
            Route::Developer => self.developer.focus_handle(cx),
            Route::Passwords => {
                // The field that matters: master password, unlock, or search.
                self.passwords
                    .update(cx, |p, cx| p.focus_primary(window, cx));
                return;
            }
            Route::Settings => self.settings.focus_handle(cx),
        };
        window.focus(&handle, cx);
    }

    fn handle_nav(&mut self, nav: Navigate, window: &mut Window, cx: &mut Context<Self>) {
        match nav {
            Navigate::To(route) => self.navigate(route, window, cx),
            Navigate::OpenNote(id) => self.open_note(id, window, cx),
            Navigate::ProjectTasks(id) => {
                self.navigate(Route::Tasks, window, cx);
                self.tasks
                    .update(cx, |tasks, cx| tasks.show_project(id, cx));
            }
        }
    }

    pub fn open_note(&mut self, id: NoteId, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Notes, window, cx);
        self.notes
            .update(cx, |notes, cx| notes.open_note(id, window, cx));
    }

    pub fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Notes, window, cx);
        self.notes
            .update(cx, |notes, cx| notes.new_note(window, cx));
    }

    pub fn new_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Tasks, window, cx);
        self.tasks
            .update(cx, |tasks, cx| tasks.focus_new_task(window, cx));
    }

    pub fn new_event(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Calendar, window, cx);
        self.calendar
            .update(cx, |calendar, cx| calendar.focus_quick_add(window, cx));
    }

    pub fn new_habit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Habits, window, cx);
        self.habits
            .update(cx, |habits, cx| habits.focus_quick_add(window, cx));
    }

    pub fn open_daily_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Notes, window, cx);
        self.notes
            .update(cx, |notes, cx| notes.open_daily_note(window, cx));
    }

    /// Asks for a name, creates the project and shows it.
    pub fn new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let library = self.library.clone();
        let shell = cx.entity().downgrade();
        ottero_ui::feedback::prompt_text(
            "New project",
            "Project name",
            "",
            "Create",
            window,
            cx,
            move |name, window, cx| {
                let result = library.update(cx, |lib, cx| lib.create_project(&name, cx));
                if let Some(id) = ottero_ui::feedback::report(result, window, cx) {
                    _ = shell.update(cx, |s, cx| {
                        s.open_target(NavTarget::Project(id), window, cx)
                    });
                }
            },
        );
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.layout
            .update(cx, |layout, cx| layout.toggle_sidebar(cx));
    }

    pub fn route(&self) -> Route {
        self.route
    }

    pub fn library(&self) -> &Entity<Library> {
        &self.library
    }

    pub fn notes(&self) -> &Entity<NotesView> {
        &self.notes
    }

    pub fn tasks(&self) -> &Entity<TasksView> {
        &self.tasks
    }

    pub fn calendar(&self) -> &Entity<CalendarView> {
        &self.calendar
    }

    pub fn habits(&self) -> &Entity<HabitsView> {
        &self.habits
    }

    pub fn developer(&self) -> &Entity<DeveloperView> {
        &self.developer
    }

    fn environment_command(
        &mut self,
        command: EnvironmentCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane = match self.route {
            Route::Developer => self.developer.read(cx).pane().clone(),
            Route::Passwords => self.passwords.read(cx).pane().clone(),
            _ => {
                ottero_ui::feedback::info_toast(
                    "Open a project secrets item in Developer or Passwords first.",
                    window,
                    cx,
                );
                return;
            }
        };
        let editor = pane.read(cx).environment_editor().clone();
        if !editor.read(cx).is_active() {
            ottero_ui::feedback::info_toast("Open a project secrets item first.", window, cx);
            return;
        }
        if matches!(
            command,
            EnvironmentCommand::Import | EnvironmentCommand::Paste
        ) && !pane.read(cx).is_editing()
        {
            ottero_ui::feedback::info_toast(
                "Edit this item before importing variables.",
                window,
                cx,
            );
            return;
        }
        if matches!(command, EnvironmentCommand::Export) && pane.read(cx).is_editing() {
            ottero_ui::feedback::info_toast("Save this item before exporting.", window, cx);
            return;
        }
        editor.update(cx, |editor, cx| match command {
            EnvironmentCommand::Import => editor.import(window, cx),
            EnvironmentCommand::Paste => editor.paste(cx),
            EnvironmentCommand::Copy => editor.copy_all(window, cx),
            EnvironmentCommand::Export => editor.export(window, cx),
        });
    }

    pub fn focus_view(&self) -> &Entity<FocusView> {
        &self.focus_view
    }

    pub fn focus_ctrl(&self) -> &Entity<FocusController> {
        &self.focus_ctrl
    }

    pub fn passwords(&self) -> &Entity<PasswordsView> {
        &self.passwords
    }

    pub fn layout(&self, cx: &App) -> LayoutState {
        self.layout.read(cx).state().clone()
    }

    /// Collapses or expands the list panel of the current screen (Notes today).
    pub fn toggle_list_panel(&mut self, cx: &mut Context<Self>) {
        self.layout.update(cx, |layout, cx| {
            layout.toggle_panel(ui_state::NOTES_LIST, cx)
        });
    }

    #[must_use]
    pub fn settings(&self) -> &Entity<SettingsView> {
        &self.settings
    }

    /// Settings → Account, optionally on a form.
    pub fn open_account(
        &mut self,
        mode: Option<crate::views::account::Mode>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate(Route::Settings, window, cx);
        if self.route() == Route::Settings {
            self.settings
                .update(cx, |settings, cx| settings.open_account(mode, window, cx));
        }
    }

    /// ⌘K toggles the palette. It never stacks: a second press closes it, and it
    /// doesn't open over another dialog (a confirmation, say).
    pub fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            if self.palette_open.get() {
                window.close_dialog(cx);
            }
            self.palette_open.set(false);
            return;
        }
        let palette_open = self.palette_open.clone();
        palette_open.set(true);
        let state = cx.new(|cx| CommandState::new(window, cx));
        let notes = self.library.read(cx).notes(false).unwrap_or_default();
        let note_ids: Vec<NoteId> = notes
            .iter()
            .take(PALETTE_NOTE_LIMIT)
            .map(|n| n.id)
            .collect();
        let note_items: Vec<CommandItem> = notes
            .into_iter()
            .take(PALETTE_NOTE_LIMIT)
            .map(|n| {
                CommandItem::new()
                    .label(n.title)
                    .icon(Lucide::FileText)
                    .keywords([n.preview])
            })
            .collect();
        let shell = cx.entity().downgrade();
        let (command_items, command_actions): (Vec<_>, Vec<_>) =
            palette_commands().into_iter().unzip();
        let command_actions = std::rc::Rc::new(command_actions);
        let search_focus = state.focus_handle(cx);

        window.open_dialog(cx, move |dialog, _, _| {
            let state = state.clone();
            let command_items = command_items.clone();
            let command_actions = command_actions.clone();
            let note_items = note_items.clone();
            let note_ids = note_ids.clone();
            let shell = shell.clone();
            let closed = palette_open.clone();
            let content_flag = palette_open.clone();
            dialog
                .on_close(move |_, _, _| closed.set(false))
                .close_button(false)
                .p_0()
                // Dialog widths are in pixels; 560 px fits the palette at every interface size.
                .w(px(560.))
                .content(move |content, _, _| {
                    let content_flag = content_flag.clone();
                    let note_ids = note_ids.clone();
                    let shell = shell.clone();
                    let command_actions = command_actions.clone();
                    let palette_flag = content_flag.clone();
                    content.child(
                        Command::new(&state)
                            .bordered(false)
                            .placeholder("Search notes or run a command…")
                            .max_h(rems(26.))
                            .group(
                                CommandGroup::new()
                                    .label("Commands")
                                    .items(command_items.clone()),
                            )
                            .when(!note_items.is_empty(), |cmd| {
                                cmd.group(
                                    CommandGroup::new().label("Notes").items(note_items.clone()),
                                )
                            })
                            .on_confirm(move |index, window, cx| {
                                // Close first: closing restores the previous focus, and
                                // commands like "New note" must be able to move it.
                                palette_flag.set(false);
                                window.close_dialog(cx);
                                if index.section == 0
                                    && let Some(action) = command_actions.get(index.row)
                                {
                                    window.dispatch_action(action.boxed_clone(), cx);
                                } else if index.section == 1
                                    && let Some(id) = note_ids.get(index.row).copied()
                                {
                                    _ = shell
                                        .update(cx, |shell, cx| shell.open_note(id, window, cx));
                                }
                            }),
                    )
                })
        });
        // Type straight away: the search field takes focus when the palette opens.
        window.focus(&search_focus, cx);
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = self.layout.read(cx).state().clone();
        let collapsed = layout.sidebar_collapsed;
        let width = px(layout.sidebar_width);

        let count_suffix = |count: usize| {
            move |_: &mut Window, cx: &mut App| {
                div().when(count > 0, |this| {
                    this.px_1p5()
                        .rounded(cx.theme().radius)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(count.to_string())
                })
            }
        };

        let preferences = self.layout.read(cx).navigation();
        let ordered = preferences.ordered();
        let hidden: Vec<_> = ordered
            .iter()
            .copied()
            .filter(|e| !preferences.is_visible(e.id))
            .collect();
        let groups: Vec<SidebarGroup<SidebarMenu>> = std::iter::once(())
            .map(|()| {
                let mut items: Vec<_> = ordered
                    .iter()
                    .copied()
                    .filter(|entry| preferences.is_visible(entry.id))
                    .map(|entry| {
                        let route = entry.route;
                        let children: Vec<SidebarMenuItem> = entry
                            .children
                            .iter()
                            .map(|child| {
                                let target = child.target;
                                SidebarMenuItem::new(child.label)
                                    .active(self.is_target_active(target, cx))
                                    .suffix(count_suffix(self.badge_count(child.badge)))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_target(target, window, cx)
                                    }))
                            })
                            .collect();
                        let mut children = children;
                        if entry.dynamic == Some(nav::DynamicChildren::Projects) {
                            for (project, open) in self.library.read(cx).projects() {
                                let target = NavTarget::Project(project.id());
                                children.push(
                                    SidebarMenuItem::new(gpui_kit::SharedString::from(
                                        project.name().to_owned(),
                                    ))
                                    .icon(Lucide::FolderKanban)
                                    .active(self.is_target_active(target, cx))
                                    .suffix(count_suffix(open))
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.open_target(target, window, cx)
                                        },
                                    )),
                                );
                            }
                            children.push(
                                SidebarMenuItem::new("New project")
                                    .icon(Lucide::Plus)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_project(window, cx)
                                    })),
                            );
                        }
                        if entry.dynamic == Some(nav::DynamicChildren::VaultTags) {
                            let tags = self.library.read(cx).vault_tags();
                            let current = self.passwords.read(cx).tag().map(str::to_owned);
                            for (tag, n) in tags.into_iter().take(SIDEBAR_TAGS) {
                                let active = self.route == Route::Passwords
                                    && current.as_deref() == Some(tag.as_str());
                                let label = gpui_kit::SharedString::from(format!("#{tag}"));
                                children.push(
                                    SidebarMenuItem::new(label)
                                        .active(active)
                                        .suffix(count_suffix(n))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.navigate(Route::Passwords, window, cx);
                                            let tag = tag.clone();
                                            this.passwords.update(cx, |p, cx| {
                                                p.set_tag(Some(tag), window, cx)
                                            });
                                        })),
                                );
                            }
                        }
                        let has_children = !children.is_empty();
                        SidebarMenuItem::new(entry.label)
                            .icon(entry.icon)
                            .active(self.route == route && !has_children)
                            .default_open(self.route == route)
                            .suffix(count_suffix(self.badge_count(entry.badge)))
                            .children(children)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.navigate(route, window, cx)
                            }))
                    })
                    .collect();
                if !hidden.is_empty() {
                    let mut children: Vec<_> = hidden
                        .iter()
                        .map(|entry| {
                            let action = entry.action;
                            SidebarMenuItem::new(entry.label)
                                .icon(entry.icon)
                                .active(self.route == entry.route)
                                .on_click(move |_, w, cx| w.dispatch_action(action(), cx))
                        })
                        .collect();
                    children.push(
                        SidebarMenuItem::new("Customize sidebar…")
                            .icon(Lucide::Settings)
                            .on_click(|_, w, cx| w.dispatch_action(Box::new(CustomizeSidebar), cx)),
                    );
                    items.push(
                        SidebarMenuItem::new("More")
                            .accessibility_label("More features")
                            .icon(Lucide::Ellipsis)
                            .click_to_toggle(!collapsed)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if collapsed {
                                    this.toggle_sidebar(cx);
                                }
                            }))
                            .children(children),
                    );
                }
                SidebarGroup::new("Workspace").child(SidebarMenu::new().children(items))
            })
            .collect();

        // Settings: a single icon button in the footer (name in its tooltip,
        // with the shortcut). Also reachable via ⌘, and the command palette.
        // Only when Ottero Cloud is locked or has an error, show one quiet
        // action next to Settings so sync never stalls silently.
        let lib = self.library.read(cx);
        let cloud_signed_in = matches!(
            lib.account().state(),
            ottero_core::account::AccountState::SignedIn { plan, .. } if plan.has_cloud()
        );
        let sync_locked = cloud_signed_in && !lib.account().is_unlocked();
        let sync_error = cloud_signed_in && !sync_locked && lib.sync_last_error().is_some();
        let sync_notice: Option<Button> = if sync_locked {
            let mut btn = Button::new("sidebar-sync-locked")
                .ghost()
                .small()
                .icon(Lucide::LockKeyhole)
                .tooltip("Ottero Cloud is locked — click to unlock sync")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_account(None, window, cx);
                }));
            if !collapsed {
                btn = btn.label("Unlock sync");
            }
            Some(btn)
        } else if sync_error {
            let mut btn = Button::new("sidebar-sync-issue")
                .ghost()
                .small()
                .icon(Lucide::Cloud)
                .tooltip("Sync issue — click to view in Account")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_account(None, window, cx);
                }));
            if !collapsed {
                btn = btn.label("Sync issue");
            }
            Some(btn)
        } else {
            None
        };
        let settings_item = h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_1()
            .child(
                Button::new("sidebar-settings")
                    .ghost()
                    .small()
                    .icon(Lucide::Settings)
                    .tooltip_with_action("Settings", &GoSettings, None)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.navigate(Route::Settings, window, cx)
                    })),
            )
            .children(sync_notice);

        // The sidebar animates collapse/expand between widths it has seen
        // before. Keying it by width means a drag starts a fresh state at the
        // new width every frame, so resizing tracks the pointer with no easing lag.
        let sidebar = Sidebar::new(gpui_kit::SharedString::from(format!(
            "main-sidebar-{}",
            layout.sidebar_width.round() as i32
        )))
        .collapsible(SidebarCollapsible::Icon)
        .collapsed(collapsed)
        .w(width)
        .header(self.search_entry(collapsed, cx))
        .children(groups)
        .footer(settings_item);

        div()
            .relative()
            .h_full()
            .flex_shrink_0()
            .on_drag_move(
                cx.listener(|this, e: &gpui_kit::DragMoveEvent<ResizeSidebar>, _, cx| {
                    let width = f32::from(e.event.position.x - e.bounds.left());
                    this.set_sidebar_width(width, cx);
                }),
            )
            .child(sidebar)
            .when(!collapsed, |this| {
                this.child(
                    resize_handle("sidebar-edge", gpui_kit::Axis::Horizontal)
                        .inside(HandleEdge::Trailing)
                        .on_drag(ResizeSidebar, |drag, _, _, cx| cx.new(|_| drag.clone()))
                        .on_double_click(cx.listener(|this, _, _, cx| {
                            this.set_sidebar_width(LayoutState::DEFAULT_SIDEBAR, cx);
                        })),
                )
            })
    }

    /// "Search ⌘K" at the top of the sidebar: the visible way into the command
    /// palette. It is the sidebar header itself, so its hover covers just this
    /// row. (The app name lives in the menu bar and Dock; the sidebar keeps its
    /// space for navigation.)
    fn search_entry(&self, collapsed: bool, cx: &mut Context<Self>) -> SidebarHeader {
        let theme = cx.theme();
        let shortcut = crate::actions::shortcut_for(&OpenCommandPalette)
            .and_then(|k| Keystroke::parse(k).ok());
        SidebarHeader::new()
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, window, cx| this.open_command_palette(window, cx)),
            )
            .child(
                h_flex()
                    .id("open-palette")
                    .w_full()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(Lucide::Search).small())
                    .when(!collapsed, |this| {
                        this.child(div().flex_1().child("Search"))
                            .children(shortcut.map(|k| Kbd::new(k).appearance(false)))
                    })
                    .test_support(),
            )
    }

    fn title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_focus = (self.route != Route::Focus)
            .then(|| {
                let ctrl = self.focus_ctrl.read(cx);
                let t = ctrl.timer();
                t.is_active().then(|| {
                    let remaining =
                        ottero_core::format_mm_ss(t.remaining_secs(ottero_core::clock::now()));
                    let running = t.is_running();
                    let label = if t.label.is_empty() {
                        t.kind.label().to_owned()
                    } else {
                        t.label.clone()
                    };
                    (remaining, running, label)
                })
            })
            .flatten();

        TitleBar::new().child(
            h_flex()
                .w_full()
                .pr_2()
                .items_center()
                .justify_between()
                // Panel toggles live here so they never move when a panel collapses.
                // `occlude` keeps the title bar's drag area out from under them:
                // Windows hit-tests every hitbox under the pointer, and without
                // this it reports "caption" and the OS swallows the click.
                .child(
                    h_flex()
                        .id("title-bar-toggles")
                        .occlude()
                        .gap_1()
                        .child(
                            Button::new("toggle-sidebar")
                                .ghost()
                                .xsmall()
                                .icon(Lucide::PanelLeft)
                                .tooltip_with_action("Toggle sidebar", &ToggleSidebar, None)
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                        )
                        .when(self.route == Route::Notes, |this| {
                            this.child(
                                Button::new("toggle-list-panel")
                                    .ghost()
                                    .xsmall()
                                    .icon(Lucide::PanelLeftDashed)
                                    .tooltip_with_action("Toggle note list", &ToggleListPanel, None)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_list_panel(cx)),
                                    ),
                            )
                        }),
                )
                .when_some(active_focus, |bar, (remaining, running, label)| {
                    let theme = cx.theme();
                    bar.child(
                        h_flex()
                            .id("title-bar-focus-pill")
                            .occlude()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new("focus-pill-open")
                                    .ghost()
                                    .xsmall()
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_1p5()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .size(rems(0.375))
                                                    .flex_shrink_0()
                                                    .rounded_full()
                                                    .bg(if running {
                                                        theme.primary
                                                    } else {
                                                        theme.muted_foreground
                                                    }),
                                            )
                                            .child(div().flex_shrink_0().text_xs().child(remaining))
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .max_w(rems(14.))
                                                    .truncate()
                                                    .text_xs()
                                                    .text_color(theme.muted_foreground)
                                                    .child(label),
                                            ),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.navigate(Route::Focus, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("focus-pill-toggle")
                                    .ghost()
                                    .xsmall()
                                    .icon(if running { Lucide::Pause } else { Lucide::Play })
                                    .tooltip(if running {
                                        "Pause focus"
                                    } else {
                                        "Resume focus"
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.focus_ctrl.update(cx, |f, cx| f.toggle(window, cx));
                                    })),
                            ),
                    )
                }),
        )
    }
}

/// Commands shown in the palette. Each carries its Action, so the palette also
/// shows the shortcut.
fn palette_commands() -> Vec<(CommandItem, Box<dyn Action>)> {
    let cmd = |label: &'static str, icon: Lucide, action: Box<dyn Action>, keywords: &[&str]| {
        let shortcut = crate::actions::shortcut_for(&*action);
        // A custom row so the shortcut hint shows without attaching the Action:
        // the palette runs the command itself, after it has closed (see on_confirm).
        let item = CommandItem::new()
            .label(label)
            .icon(icon)
            .keywords(keywords.iter().map(|k| (*k).to_owned()).collect::<Vec<_>>())
            .child(move |_, cx| {
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(icon)
                            .small()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(div().flex_1().child(label))
                    .children(
                        shortcut
                            .and_then(|s| Keystroke::parse(s).ok())
                            .map(|k| Kbd::new(k).appearance(false)),
                    )
            });
        (item, action)
    };
    vec![
        cmd(
            "New note",
            Lucide::SquarePen,
            Box::new(NewNote),
            &["create", "write"],
        ),
        cmd(
            "New task",
            Lucide::Plus,
            Box::new(NewTask),
            &["create", "todo"],
        ),
        cmd(
            "New event",
            Lucide::CalendarPlus,
            Box::new(NewEvent),
            &["create", "calendar", "meeting", "appointment"],
        ),
        cmd(
            "New habit",
            Lucide::Sprout,
            Box::new(NewHabit),
            &["create", "routine", "streak"],
        ),
        cmd(
            "Go to Today",
            Lucide::House,
            Box::new(GoToday),
            &["dashboard", "home"],
        ),
        cmd("Go to Notes", Lucide::NotebookPen, Box::new(GoNotes), &[]),
        cmd(
            "Go to Tasks",
            Lucide::ListTodo,
            Box::new(GoTasks),
            &["todo"],
        ),
        cmd(
            "Go to Calendar",
            Lucide::CalendarDays,
            Box::new(GoCalendar),
            &["events", "schedule", "agenda", "month", "week"],
        ),
        cmd(
            "Import calendar (.ics)",
            Lucide::CalendarPlus,
            Box::new(ImportCalendar),
            &[
                "ics",
                "ical",
                "google calendar",
                "apple calendar",
                "outlook",
                "events",
            ],
        ),
        cmd(
            "Show deleted events",
            Lucide::Trash,
            Box::new(ShowDeletedEvents),
            &["calendar", "trash", "restore", "undelete"],
        ),
        cmd(
            "Go to Habits",
            Lucide::Sprout,
            Box::new(GoHabits),
            &["routine", "streak", "check in"],
        ),
        cmd(
            "Go to Developer",
            Lucide::Code,
            Box::new(GoDeveloper),
            &["projects", "environment", "secrets", "env"],
        ),
        cmd(
            "New developer project",
            Lucide::FolderPlus,
            Box::new(NewDeveloperProject),
            &["developer", "project"],
        ),
        cmd(
            "Show / hide archived developer projects",
            Lucide::Archive,
            Box::new(ToggleArchivedDeveloperProjects),
            &["developer", "archive", "restore"],
        ),
        cmd(
            "New project secrets",
            Lucide::KeyRound,
            Box::new(NewProjectSecrets),
            &["developer", "env", "environment"],
        ),
        cmd(
            "Import environment file…",
            Lucide::Code,
            Box::new(ImportProjectEnvironment),
            &["dotenv", "env", "secrets"],
        ),
        cmd(
            "Paste environment variables",
            Lucide::Code,
            Box::new(PasteProjectEnvironment),
            &["dotenv", "env", "secrets"],
        ),
        cmd(
            "Copy environment as .env",
            Lucide::Copy,
            Box::new(CopyProjectEnvironment),
            &["dotenv", "env", "secrets"],
        ),
        cmd(
            "Export environment file…",
            Lucide::Code,
            Box::new(ExportProjectEnvironment),
            &["dotenv", "env", "secrets"],
        ),
        cmd(
            "Go to Focus",
            Lucide::Timer,
            Box::new(GoFocus),
            &["pomodoro", "timer", "deep work"],
        ),
        cmd(
            "Start / pause focus",
            Lucide::Play,
            Box::new(FocusToggle),
            &["pomodoro", "timer"],
        ),
        cmd(
            "Reset focus timer",
            Lucide::RotateCcw,
            Box::new(FocusReset),
            &["pomodoro", "timer"],
        ),
        cmd(
            "Finish focus session",
            Lucide::Check,
            Box::new(FocusFinish),
            &["pomodoro", "timer", "log"],
        ),
        cmd(
            "Go to Passwords",
            Lucide::KeyRound,
            Box::new(GoPasswords),
            &["vault", "password manager", "logins", "cards"],
        ),
        cmd(
            "New login",
            Lucide::KeyRound,
            Box::new(NewLogin),
            &["password", "vault", "account", "create"],
        ),
        cmd(
            "Generate password",
            Lucide::Dices,
            Box::new(GeneratePassword),
            &["random", "passphrase", "vault"],
        ),
        cmd(
            "Lock vault",
            Lucide::LockKeyhole,
            Box::new(LockVault),
            &["passwords", "secure"],
        ),
        cmd(
            "Change master password",
            Lucide::KeyRound,
            Box::new(ChangeMasterPassword),
            &["vault", "passwords", "security"],
        ),
        cmd(
            "Use Ottero account for Passwords…",
            Lucide::Cloud,
            Box::new(UseAccountForPasswords),
            &["vault", "link", "master password", "cloud", "one password"],
        ),
        cmd(
            "Create new recovery key",
            Lucide::KeyRound,
            Box::new(CreateRecoveryKey),
            &["vault", "emergency kit", "forgot password"],
        ),
        cmd(
            "Export vault backup",
            Lucide::Download,
            Box::new(ExportVaultBackup),
            &["vault", "passwords", "encrypted", "save", "copy"],
        ),
        cmd(
            "Restore vault backup",
            Lucide::Upload,
            Box::new(RestoreVaultBackup),
            &["vault", "passwords", "import", "open backup"],
        ),
        cmd(
            "Show archived vaults",
            Lucide::Archive,
            Box::new(ShowVaultArchives),
            &["vault", "old vault", "restore"],
        ),
        cmd(
            "What’s new",
            Lucide::Info,
            Box::new(crate::actions::ShowChangelog),
            &["changelog", "release notes", "changes", "updates"],
        ),
        cmd(
            "Check for updates",
            Lucide::RefreshCw,
            Box::new(CheckForUpdates),
            &["update", "new version", "upgrade"],
        ),
        cmd(
            "Open settings",
            Lucide::Settings,
            Box::new(GoSettings),
            &["preferences"],
        ),
        cmd(
            "Open account settings",
            Lucide::CircleUser,
            Box::new(crate::actions::OpenAccountSettings),
            &["account", "ottero cloud", "profile", "devices", "plan"],
        ),
        cmd(
            "Sync Ottero Cloud now",
            Lucide::Cloud,
            Box::new(crate::actions::SyncNow),
            &["sync", "cloud", "push", "pull", "backup"],
        ),
        cmd(
            "Create Ottero Cloud account",
            Lucide::Cloud,
            Box::new(crate::actions::CreateAccount),
            &["account", "sign up", "register", "cloud", "sync"],
        ),
        cmd(
            "Sign in to Ottero Cloud",
            Lucide::Cloud,
            Box::new(crate::actions::SignInToCloud),
            &["account", "log in", "login", "cloud", "sync"],
        ),
        cmd(
            "Sign out of Ottero Cloud…",
            Lucide::Cloud,
            Box::new(crate::actions::SignOutOfCloud),
            &["account", "log out", "logout", "cloud"],
        ),
        cmd(
            "Copy Secret Key",
            Lucide::KeyRound,
            Box::new(crate::actions::CopySecretKey),
            &["account", "cloud", "secret key", "new device"],
        ),
        cmd(
            "Save Emergency Kit…",
            Lucide::Download,
            Box::new(crate::actions::SaveEmergencyKit),
            &["account", "cloud", "secret key", "recovery", "print"],
        ),
        cmd(
            "Turn on two-step verification…",
            Lucide::ShieldCheck,
            Box::new(crate::actions::TurnOnTwoStep),
            &[
                "account",
                "cloud",
                "2fa",
                "two-factor",
                "authenticator",
                "security",
            ],
        ),
        cmd(
            "New backup codes…",
            Lucide::ShieldCheck,
            Box::new(crate::actions::NewBackupCodes),
            &[
                "account",
                "cloud",
                "2fa",
                "two-factor",
                "two-step",
                "recovery",
            ],
        ),
        cmd(
            "Turn off two-step verification…",
            Lucide::ShieldCheck,
            Box::new(crate::actions::TurnOffTwoStep),
            &[
                "account",
                "cloud",
                "2fa",
                "two-factor",
                "authenticator",
                "security",
            ],
        ),
        cmd(
            "Customize sidebar",
            Lucide::PanelLeft,
            Box::new(CustomizeSidebar),
            &[
                "workspace",
                "sidebar",
                "hide",
                "show",
                "reorder",
                "features",
            ],
        ),
        cmd(
            "Toggle sidebar",
            Lucide::PanelLeft,
            Box::new(ToggleSidebar),
            &[],
        ),
        cmd(
            "Toggle note list",
            Lucide::PanelLeftDashed,
            Box::new(ToggleListPanel),
            &["notes", "panel", "focus"],
        ),
        cmd(
            "Find in note",
            Lucide::Search,
            Box::new(FindInNote),
            &["search", "replace", "match"],
        ),
        cmd(
            "Show or hide Markdown source",
            Lucide::Code,
            Box::new(ToggleMarkdownSource),
            &["raw", "markdown", "source", "preview"],
        ),
        cmd(
            "Use light theme",
            Lucide::Sun,
            Box::new(UseLightTheme),
            &["appearance"],
        ),
        cmd(
            "Use dark theme",
            Lucide::Moon,
            Box::new(UseDarkTheme),
            &["appearance"],
        ),
        cmd(
            "Use system theme",
            Lucide::SunMoon,
            Box::new(UseSystemTheme),
            &["appearance"],
        ),
    ]
}

/// Runs `f` on the main shell, if it is still open.
///
/// Deferred: app-level action handlers run while the window is still borrowed
/// by the event dispatch, so updating it directly would silently fail.
/// Unique ID so update status changes replace the existing update toast
/// instead of stacking duplicates.
struct UpdateNotice;

/// The notice for an update status change (nothing for the quiet ones).
fn update_notice(status: &crate::updates::Status, window: &mut Window, cx: &mut App) {
    use crate::updates::Status;
    use gpui_kit::component::notification::{Notification, NotificationType};
    let notification = match status {
        Status::Ready { version, .. } => Notification::new()
            .id::<UpdateNotice>()
            .message(format!("Ottero {version} is ready to install."))
            .action(|_, _, cx| {
                Button::new("update-restart")
                    .outline()
                    .xsmall()
                    .label("Restart")
                    .on_click(cx.listener(|this, _, window, cx| {
                        window.dispatch_action(Box::new(RestartToUpdate), cx);
                        this.dismiss(window, cx);
                    }))
            })
            .autohide(false),
        Status::Available { version } => Notification::new()
            .id::<UpdateNotice>()
            .message(format!("Ottero {version} is available on GitHub.")),
        Status::Unsupported if crate::updates::manual_pending(cx) => Notification::new()
            .id::<UpdateNotice>()
            .message("Automatic updates aren't available for this platform."),
        Status::UpToDate if crate::updates::manual_pending(cx) => {
            Notification::new().id::<UpdateNotice>().message(format!(
                "Ottero {} is up to date.",
                env!("CARGO_PKG_VERSION")
            ))
        }
        Status::Failed { message } => Notification::new()
            .id::<UpdateNotice>()
            .with_type(NotificationType::Error)
            .message(message.clone()),
        _ => return,
    };
    window.push_notification(notification, cx);
}

fn with_shell(
    cx: &mut App,
    f: impl FnOnce(&mut AppShell, &mut Window, &mut Context<AppShell>) + 'static,
) {
    let Some(handle) = cx.try_global::<ShellHandle>() else {
        return;
    };
    let (window, shell) = (handle.window, handle.shell.clone());
    cx.defer(move |cx| {
        let updated = window.update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| f(shell, window, cx))
        });
        if updated.is_err() {
            eprintln!("ottero: command ignored, main window is unavailable");
        }
    });
}

/// App-level command handlers. Call once at startup.
pub fn register_commands(cx: &mut App) {
    cx.on_action(|_: &OpenCommandPalette, cx| {
        with_shell(cx, |s, w, cx| s.open_command_palette(w, cx))
    });
    cx.on_action(|_: &NewNote, cx| with_shell(cx, |s, w, cx| s.new_note(w, cx)));
    cx.on_action(|_: &NewTask, cx| with_shell(cx, |s, w, cx| s.new_task(w, cx)));
    cx.on_action(|_: &GoToday, cx| with_shell(cx, |s, w, cx| s.navigate(Route::Today, w, cx)));
    cx.on_action(|_: &GoNotes, cx| with_shell(cx, |s, w, cx| s.navigate(Route::Notes, w, cx)));
    cx.on_action(|_: &GoTasks, cx| with_shell(cx, |s, w, cx| s.navigate(Route::Tasks, w, cx)));
    cx.on_action(|_: &GoCalendar, cx| {
        with_shell(cx, |s, w, cx| s.navigate(Route::Calendar, w, cx))
    });
    cx.on_action(|_: &NewEvent, cx| with_shell(cx, |s, w, cx| s.new_event(w, cx)));
    cx.on_action(|_: &ImportCalendar, cx| {
        with_shell(cx, |s, w, cx| {
            crate::views::calendar::import_ics_file(s.library.clone(), w, cx)
        })
    });
    cx.on_action(|_: &ShowDeletedEvents, cx| {
        with_shell(cx, |s, w, cx| {
            crate::views::event_trash::open_event_trash(s.library.clone(), w, cx)
        })
    });
    cx.on_action(|_: &GoHabits, cx| with_shell(cx, |s, w, cx| s.navigate(Route::Habits, w, cx)));
    cx.on_action(|_: &NewHabit, cx| with_shell(cx, |s, w, cx| s.new_habit(w, cx)));
    cx.on_action(|_: &GoDeveloper, cx| {
        with_shell(cx, |s, w, cx| s.navigate(Route::Developer, w, cx))
    });
    cx.on_action(|_: &NewDeveloperProject, cx| {
        with_shell(cx, |s, w, cx| {
            s.navigate(Route::Developer, w, cx);
            s.developer.update(cx, |d, cx| d.begin_new(w, cx));
        })
    });
    cx.on_action(|_: &NewProjectSecrets, cx| {
        with_shell(cx, |s, w, cx| {
            s.navigate(Route::Developer, w, cx);
            s.developer.update(cx, |d, cx| d.new_environment(w, cx));
        })
    });
    cx.on_action(|_: &ToggleArchivedDeveloperProjects, cx| {
        with_shell(cx, |s, w, cx| {
            s.navigate(Route::Developer, w, cx);
            s.developer.update(cx, |d, cx| d.toggle_archived(w, cx));
        })
    });
    cx.on_action(|_: &ImportProjectEnvironment, cx| {
        with_shell(cx, |s, w, cx| {
            s.environment_command(EnvironmentCommand::Import, w, cx)
        })
    });
    cx.on_action(|_: &PasteProjectEnvironment, cx| {
        with_shell(cx, |s, w, cx| {
            s.environment_command(EnvironmentCommand::Paste, w, cx)
        })
    });
    cx.on_action(|_: &CopyProjectEnvironment, cx| {
        with_shell(cx, |s, w, cx| {
            s.environment_command(EnvironmentCommand::Copy, w, cx)
        })
    });
    cx.on_action(|_: &ExportProjectEnvironment, cx| {
        with_shell(cx, |s, w, cx| {
            s.environment_command(EnvironmentCommand::Export, w, cx)
        })
    });
    cx.on_action(|_: &GoFocus, cx| with_shell(cx, |s, w, cx| s.navigate(Route::Focus, w, cx)));
    cx.on_action(|_: &FocusToggle, cx| {
        with_shell(cx, |s, w, cx| {
            s.focus_ctrl.update(cx, |f, cx| f.toggle(w, cx));
        })
    });
    cx.on_action(|_: &FocusReset, cx| {
        with_shell(cx, |s, _, cx| {
            s.focus_ctrl.update(cx, |f, cx| f.reset(cx));
        })
    });
    cx.on_action(|_: &FocusFinish, cx| {
        with_shell(cx, |s, w, cx| {
            if let Err(err) = s.focus_ctrl.update(cx, |f, cx| f.finish_now(true, cx)) {
                ottero_ui::feedback::error_toast(
                    format!("Couldn't save focus session: {err}"),
                    w,
                    cx,
                );
            }
        })
    });
    cx.on_action(|_: &GoPasswords, cx| {
        with_shell(cx, |s, w, cx| s.navigate(Route::Passwords, w, cx))
    });
    cx.on_action(|_: &LockVault, cx| {
        with_shell(cx, |s, _, cx| {
            s.library.update(cx, |lib, cx| lib.lock_vault(cx))
        })
    });
    cx.on_action(|_: &NewLogin, cx| {
        with_shell(cx, |s, w, cx| {
            s.navigate(Route::Passwords, w, cx);
            s.passwords.update(cx, |p, cx| p.new_login(w, cx));
        })
    });
    cx.on_action(|_: &ChangeMasterPassword, cx| {
        with_shell(cx, |s, w, cx| {
            crate::views::passwords::open_change_password(s.library.clone(), w, cx)
        })
    });
    cx.on_action(|_: &UseAccountForPasswords, cx| {
        with_shell(cx, |s, w, cx| {
            use ottero_core::vault::lock::{LockState, Opener};
            let lib = s.library.read(cx);
            let signed_in = lib.account().signed_in_email();
            let open = lib.account().is_unlocked();
            let (state, opener) = (lib.vault_state(), lib.vault_opener());
            if let Some(email) = lib
                .vault_linked_email()
                .filter(|_| opener == Some(Opener::Account))
            {
                let message = format!("Passwords already opens with {email}.");
                ottero_ui::feedback::info_toast(message, w, cx);
                s.navigate(Route::Passwords, w, cx);
                return;
            }
            if signed_in.is_none() {
                s.open_account(Some(crate::views::account::Mode::SignIn), w, cx);
                ottero_ui::feedback::info_toast("Sign in to Ottero Cloud first.", w, cx);
                return;
            }
            if !open && state != LockState::Uninitialized {
                s.open_account(None, w, cx);
                ottero_ui::feedback::info_toast(
                    "Unlock your Ottero account, then Passwords asks for its master password once.",
                    w,
                    cx,
                );
                return;
            }
            s.navigate(Route::Passwords, w, cx);
            if state == LockState::Unlocked {
                let passwords = s.passwords.downgrade();
                crate::views::passwords::open_link_dialog(s.library.clone(), passwords, w, cx);
            }
        })
    });
    cx.on_action(|_: &CreateRecoveryKey, cx| {
        with_shell(cx, |s, w, cx| {
            if s.library.read(cx).vault_state() != ottero_core::vault::lock::LockState::Unlocked {
                ottero_ui::feedback::error_toast("Unlock the vault first.", w, cx);
                return;
            }
            s.navigate(Route::Passwords, w, cx);
            let task = s.library.update(cx, |lib, cx| lib.new_recovery_key(cx));
            let passwords = s.passwords.downgrade();
            w.spawn(cx, async move |cx| {
                let result = task.await;
                _ = cx.update(|window, cx| match result {
                    Ok(key) => {
                        _ = passwords.update(cx, |p, cx| p.show_recovery_key(key, cx));
                    }
                    Err(err) => ottero_ui::feedback::error_toast(format!("{err:#}"), window, cx),
                });
            })
            .detach();
        })
    });
    cx.on_action(|_: &ShowVaultArchives, cx| {
        with_shell(cx, |s, w, cx| {
            crate::views::vault_recovery::open_archives(s.library.clone(), w, cx)
        })
    });
    cx.on_action(|_: &ExportVaultBackup, cx| {
        with_shell(cx, |s, w, cx| {
            crate::views::vault_recovery::export_backup(s.library.clone(), w, cx)
        })
    });
    cx.on_action(|_: &RestoreVaultBackup, cx| {
        with_shell(cx, |s, w, cx| {
            crate::views::vault_recovery::restore_backup(s.library.clone(), w, cx)
        })
    });
    cx.on_action(|_: &GeneratePassword, cx| {
        with_shell(cx, |s, w, cx| {
            let library = s.library.clone();
            crate::views::vault_generator::open_generator(None, &library, w, cx);
        })
    });
    cx.on_action(|_: &CheckForUpdates, cx| crate::updates::check(true, cx));
    cx.on_action(|_: &RestartToUpdate, cx| {
        with_shell(cx, |s, w, cx| {
            if !s.prepare_update_restart(w, cx) {
                return;
            }
            let snapshot = crate::updates::snapshot_path(cx)
                .and_then(|path| s.library.read(cx).backup_for_update(&path));
            if snapshot.is_err() {
                ottero_ui::feedback::error_toast(
                    "Couldn't create a recovery snapshot. The update was not installed.",
                    w,
                    cx,
                );
                return;
            }
            match crate::updates::apply(cx) {
                Ok(true) => cx.quit(),
                Ok(false) => {}
                Err(err) => ottero_ui::feedback::error_toast(format!("{err:#}"), w, cx),
            }
        })
    });
    cx.on_action(|_: &crate::actions::ShowChangelog, cx| {
        with_shell(cx, |s, w, cx| {
            s.navigate(Route::Settings, w, cx);
            if s.route() == Route::Settings {
                s.settings
                    .update(cx, |settings, cx| settings.open_changelog(cx));
            }
        })
    });
    cx.on_action(|_: &GoSettings, cx| {
        with_shell(cx, |s, w, cx| s.navigate(Route::Settings, w, cx))
    });
    cx.on_action(|_: &crate::actions::OpenAccountSettings, cx| {
        with_shell(cx, |s, w, cx| s.open_account(None, w, cx))
    });
    cx.on_action(|_: &crate::actions::SyncNow, cx| {
        with_shell(cx, |s, _, cx| {
            s.library.update(cx, Library::sync_now);
        })
    });
    cx.on_action(|_: &crate::actions::CreateAccount, cx| {
        with_shell(cx, |s, w, cx| {
            s.open_account(Some(crate::views::account::Mode::SignUp), w, cx)
        })
    });
    cx.on_action(|_: &crate::actions::SignInToCloud, cx| {
        with_shell(cx, |s, w, cx| {
            s.open_account(Some(crate::views::account::Mode::SignIn), w, cx)
        })
    });
    cx.on_action(|_: &crate::actions::SignOutOfCloud, cx| {
        with_shell(cx, |s, w, cx| {
            s.open_account(None, w, cx);
            let account = s.settings.read(cx).account().clone();
            account.update(cx, |a, cx| a.confirm_sign_out(w, cx));
        })
    });
    cx.on_action(|_: &crate::actions::CopySecretKey, cx| {
        with_shell(cx, |s, w, cx| {
            s.open_account(None, w, cx);
            let account = s.settings.read(cx).account().clone();
            account.update(cx, |a, cx| a.show_secret_key(true, w, cx));
        })
    });
    cx.on_action(|_: &crate::actions::SaveEmergencyKit, cx| {
        with_shell(cx, |s, w, cx| {
            s.open_account(None, w, cx);
            let account = s.settings.read(cx).account().clone();
            account.update(cx, |a, cx| a.save_kit(w, cx));
        })
    });
    fn manage_two_step(purpose: crate::views::account::TwoStepPurpose, cx: &mut App) {
        with_shell(cx, move |s, w, cx| {
            s.open_account(None, w, cx);
            let account = s.settings.read(cx).account().clone();
            account.update(cx, |a, cx| a.manage_two_step(purpose, w, cx));
        });
    }
    cx.on_action(|_: &crate::actions::TurnOnTwoStep, cx| {
        manage_two_step(crate::views::account::TwoStepPurpose::TurnOn, cx);
    });
    cx.on_action(|_: &crate::actions::NewBackupCodes, cx| {
        manage_two_step(crate::views::account::TwoStepPurpose::NewBackupCodes, cx);
    });
    cx.on_action(|_: &crate::actions::TurnOffTwoStep, cx| {
        manage_two_step(crate::views::account::TwoStepPurpose::TurnOff, cx);
    });
    cx.on_action(|_: &CustomizeSidebar, cx| {
        with_shell(cx, |s, w, cx| {
            s.settings
                .update(cx, |settings, cx| settings.open_workspace(cx));
            s.navigate(Route::Settings, w, cx);
        })
    });
    cx.on_action(|_: &ToggleSidebar, cx| with_shell(cx, |s, _, cx| s.toggle_sidebar(cx)));
    cx.on_action(|_: &ToggleListPanel, cx| {
        with_shell(cx, |s, w, cx| {
            // The list belongs to Notes: show the screen it toggles.
            if s.route() != Route::Notes {
                s.navigate(Route::Notes, w, cx);
            }
            s.toggle_list_panel(cx);
        });
    });
    cx.on_action(|_: &OpenDailyNote, cx| with_shell(cx, |s, w, cx| s.open_daily_note(w, cx)));
    cx.on_action(|_: &FindInNote, cx| {
        // Find belongs to the open note; other screens have their own search.
        with_shell(cx, |s, w, cx| {
            if s.route() == Route::Notes {
                s.notes.update(cx, |n, cx| n.open_find(w, cx));
            }
        })
    });
    cx.on_action(|_: &FindNext, cx| {
        with_shell(cx, |s, w, cx| {
            if s.route() == Route::Notes {
                s.notes.update(cx, |n, cx| n.step_find(1, w, cx));
            }
        })
    });
    cx.on_action(|_: &FindPrevious, cx| {
        with_shell(cx, |s, w, cx| {
            if s.route() == Route::Notes {
                s.notes.update(cx, |n, cx| n.step_find(-1, w, cx));
            }
        })
    });
    cx.on_action(|_: &ToggleMarkdownSource, cx| {
        with_shell(cx, |s, w, cx| {
            if s.route() == Route::Notes {
                s.notes.update(cx, |n, cx| n.toggle_source_mode(w, cx));
            }
        })
    });
    cx.on_action(|_: &NewProject, cx| with_shell(cx, |s, w, cx| s.new_project(w, cx)));
    cx.on_action(|_: &ImportNotes, cx| {
        with_shell(cx, |s, w, cx| {
            s.navigate(Route::Notes, w, cx);
            s.notes.update(cx, |n, cx| n.import_notes(w, cx));
        })
    });
    cx.on_action(|_: &ExportNotes, cx| {
        with_shell(cx, |s, w, cx| {
            s.notes.update(cx, |n, cx| n.export_notes(w, cx))
        })
    });
    cx.on_action(|_: &UseLightTheme, cx| {
        appearance::update(cx, |s| s.theme = ThemePreference::Light)
    });
    cx.on_action(|_: &UseDarkTheme, cx| {
        appearance::update(cx, |s| s.theme = ThemePreference::Dark)
    });
    cx.on_action(|_: &UseSystemTheme, cx| {
        appearance::update(cx, |s| s.theme = ThemePreference::System)
    });
}

impl Render for AppShell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content: AnyView = match self.route {
            Route::Today => self.today.clone().into(),
            Route::Notes => self.notes.clone().into(),
            Route::Tasks => self.tasks.clone().into(),
            Route::Calendar => self.calendar.clone().into(),
            Route::Habits => self.habits.clone().into(),
            Route::Focus => self.focus_view.clone().into(),
            Route::Developer => self.developer.clone().into(),
            Route::Passwords => self.passwords.clone().into(),
            Route::Settings => self.settings.clone().into(),
        };
        let theme = cx.theme();
        v_flex()
            .track_focus(&self.focus)
            // Pointer activity also postpones auto-lock (no re-render).
            .on_mouse_move(cx.listener(|this, _, _, cx| {
                this.library.update(cx, |lib, _| lib.touch_vault());
            }))
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .child(self.title_bar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.sidebar(cx))
                    .child(div().flex_1().min_w_0().h_full().child(content)),
            )
    }
}
