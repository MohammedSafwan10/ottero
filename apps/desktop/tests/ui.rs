#![allow(clippy::expect_used)] // Test helpers outside #[test] fns.

//! UI integration tests: the real Ottero shell in a headless window.
//!
//! Each test starts the app exactly like `main` does (`ottero_desktop::start`)
//! with a temporary data folder and an in-memory database, then drives it with
//! actions, typing and key presses, and checks the stored result.
//!
//! Run: `cargo test -p ottero-desktop --test ui`

use std::time::Duration;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use ottero_core::TaskStatus;
use ottero_desktop::actions::{
    FocusToggle, GoFocus, GoNotes, GoSettings, GoTasks, GoToday, NewNote, NewTask,
    OpenCommandPalette, ToggleSidebar, UseDarkTheme, UseLightTheme,
};
use ottero_desktop::paths::AppPaths;
use ottero_desktop::shell::AppShell;
use ottero_desktop::views::Route;
use ottero_desktop::views::tasks::TaskTab;
use ottero_storage::{Store, TaskFilter};

struct App {
    window: AnyWindowHandle,
    shell: Entity<AppShell>,
    _dir: tempfile::TempDir,
    paths: AppPaths,
}

fn launch(cx: &mut TestAppContext) -> App {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = AppPaths::in_dir(dir.path().to_path_buf());
    std::fs::create_dir_all(dir.path().join("updates")).expect("update prefs directory");
    std::fs::write(
        dir.path().join("updates/prefs.json"),
        br#"{"automatic":false}"#,
    )
    .expect("offline test prefs");
    let store = Store::open_in_memory().expect("store");
    // Never the network: a local account server may be running.
    let (window, shell) = cx.update(|cx| {
        ottero_desktop::start_with_account_api(&paths, store, offline_account_api(), cx)
            .expect("start")
    });
    cx.run_until_parked();
    App {
        window,
        shell,
        _dir: dir,
        paths,
    }
}

fn offline_account_api() -> std::sync::Arc<dyn ottero_desktop::account::client::AccountApi> {
    let fake = ottero_desktop::account::fake::FakeAccountApi::new();
    fake.set_offline(true);
    std::sync::Arc::new(fake)
}

fn dispatch(app: &App, cx: &mut TestAppContext, action: impl gpui_kit::Action) {
    cx.update_window(app.window, |_, window, cx| {
        window.dispatch_action(Box::new(action), cx)
    })
    .expect("window");
    cx.run_until_parked();
}

fn type_text(app: &App, cx: &mut TestAppContext, text: &str) {
    cx.update_window(app.window, |_, window, cx| window.input(text, cx))
        .expect("window");
    cx.run_until_parked();
}

fn press(app: &App, cx: &mut TestAppContext, key: &str) {
    cx.update_window(app.window, |_, window, cx| window.press(key, cx))
        .expect("window");
    cx.run_until_parked();
}

fn route(app: &App, cx: &mut TestAppContext) -> Route {
    cx.read(|cx| app.shell.read(cx).route())
}

#[gpui_kit::test]
fn starts_on_today_and_navigates_with_actions(cx: &mut TestAppContext) {
    let app = launch(cx);
    assert_eq!(route(&app, cx), Route::Today);
    dispatch(&app, cx, GoNotes);
    assert_eq!(route(&app, cx), Route::Notes);
    dispatch(&app, cx, GoTasks);
    assert_eq!(route(&app, cx), Route::Tasks);
    dispatch(&app, cx, GoSettings);
    assert_eq!(route(&app, cx), Route::Settings);
    dispatch(&app, cx, GoToday);
    assert_eq!(route(&app, cx), Route::Today);
}

#[gpui_kit::test]
fn new_note_typing_autosaves_with_links(cx: &mut TestAppContext) {
    let app = launch(cx);
    // A target note, so the link has somewhere to point.
    cx.update(|cx| {
        app.shell.read(cx).library().clone().update(cx, |lib, cx| {
            let id = lib.create_note("Service log", cx).expect("create");
            lib.edit_note(id, "Service log", "Oil changed", cx)
                .expect("edit");
        });
    });

    dispatch(&app, cx, NewNote);
    assert_eq!(route(&app, cx), Route::Notes);

    // The title is focused on a new note; Enter moves to the body.
    type_text(&app, cx, "Car");
    press(&app, cx, "enter");
    type_text(&app, cx, "Insurance renews in May. See [[Service log]].");

    // Nothing is written before the autosave delay...
    let titles = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            app.shell
                .read(cx)
                .library()
                .read(cx)
                .notes(false)
                .expect("notes")
                .into_iter()
                .map(|n| n.title)
                .collect::<Vec<_>>()
        })
    };
    assert!(!titles(cx).contains(&"Car".to_owned()), "saved too early");

    // ...and everything is written after it.
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    assert!(
        titles(cx).contains(&"Car".to_owned()),
        "autosave did not run"
    );

    let backlinks = cx.read(|cx| {
        app.shell
            .read(cx)
            .library()
            .read(cx)
            .backlinks("service log")
            .expect("backlinks")
    });
    assert_eq!(backlinks.len(), 1);
    assert_eq!(backlinks[0].title, "Car");
    let body = cx.read(|cx| {
        let lib = app.shell.read(cx).library().read(cx);
        lib.note(backlinks[0].id)
            .expect("load")
            .expect("exists")
            .body()
            .to_owned()
    });
    assert_eq!(body, "Insurance renews in May. See [[Service log]].");
}

#[gpui_kit::test]
fn switching_notes_flushes_unsaved_typing(cx: &mut TestAppContext) {
    let app = launch(cx);
    dispatch(&app, cx, NewNote);
    type_text(&app, cx, "First");
    // Create another note immediately, before the autosave timer fires.
    dispatch(&app, cx, NewNote);
    let titles = cx.read(|cx| {
        app.shell
            .read(cx)
            .library()
            .read(cx)
            .notes(false)
            .expect("notes")
            .into_iter()
            .map(|n| n.title)
            .collect::<Vec<_>>()
    });
    assert!(
        titles.contains(&"First".to_owned()),
        "typing lost on switch: {titles:?}"
    );
}

#[gpui_kit::test]
fn quick_add_creates_task_due_today_and_clears_input(cx: &mut TestAppContext) {
    let app = launch(cx);
    dispatch(&app, cx, NewTask);
    assert_eq!(route(&app, cx), Route::Tasks);
    assert_eq!(
        cx.read(|cx| app.shell.read(cx).tasks().read(cx).tab()),
        TaskTab::Today
    );

    type_text(&app, cx, "Pay rent");
    press(&app, cx, "enter");
    // Blank input does nothing.
    press(&app, cx, "enter");

    let today = ottero_core::clock::today();
    let tasks = cx.read(|cx| {
        app.shell
            .read(cx)
            .library()
            .read(cx)
            .tasks(TaskFilter::Today(today))
            .expect("tasks")
    });
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title(), "Pay rent");
    assert_eq!(tasks[0].due(), Some(today));

    // Completing it moves it out of Today and into Done.
    let id = tasks[0].id();
    cx.update(|cx| {
        app.shell
            .read(cx)
            .library()
            .clone()
            .update(cx, |lib, cx| lib.toggle_task(id, cx))
            .expect("toggle");
    });
    let lib_tasks = |filter, cx: &mut TestAppContext| {
        cx.read(|cx| {
            app.shell
                .read(cx)
                .library()
                .read(cx)
                .tasks(filter)
                .expect("tasks")
        })
    };
    assert!(lib_tasks(TaskFilter::Today(today), cx).is_empty());
    let done = lib_tasks(TaskFilter::Closed, cx);
    assert_eq!(done[0].status(), TaskStatus::Done);
}

#[gpui_kit::test]
fn sidebar_toggle_persists_layout(cx: &mut TestAppContext) {
    let app = launch(cx);
    assert!(!cx.read(|cx| app.shell.read(cx).layout(cx).sidebar_collapsed));
    dispatch(&app, cx, ToggleSidebar);
    assert!(cx.read(|cx| app.shell.read(cx).layout(cx).sidebar_collapsed));

    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    let saved = ottero_desktop::ui_state::LayoutState::load(
        &ottero_desktop::ui_state::layout_file(&app.paths.data_dir),
    );
    assert!(saved.sidebar_collapsed, "collapsed state written to disk");
}

#[gpui_kit::test]
fn theme_commands_switch_mode_and_save(cx: &mut TestAppContext) {
    let app = launch(cx);
    dispatch(&app, cx, UseDarkTheme);
    assert_eq!(cx.read(|cx| Theme::global(cx).mode), ThemeMode::Dark);
    assert_eq!(
        cx.read(|cx| Theme::global(cx).theme_name().to_string()),
        ottero_ui::theme::DARK
    );
    dispatch(&app, cx, UseLightTheme);
    assert_eq!(cx.read(|cx| Theme::global(cx).mode), ThemeMode::Light);

    let saved = ottero_ui::appearance::AppearanceSettings::load(&app.paths.appearance);
    assert_eq!(saved.theme, ottero_ui::appearance::ThemePreference::Light);
}

#[gpui_kit::test]
fn command_palette_opens_and_escape_closes_it(cx: &mut TestAppContext) {
    let app = launch(cx);
    let dialog_open = |cx: &mut TestAppContext| {
        cx.update_window(app.window, |_, window, cx| {
            gpui_kit::component::WindowExt::has_active_dialog(window, cx)
        })
        .expect("window")
    };
    assert!(!dialog_open(cx));
    dispatch(&app, cx, OpenCommandPalette);
    assert!(dialog_open(cx));
    press(&app, cx, "escape");
    assert!(!dialog_open(cx));
}

fn click(app: &App, cx: &mut TestAppContext, id: &'static str) {
    cx.update_window(app.window, |_, window, cx| window.click(id, cx))
        .expect("window");
    cx.run_until_parked();
}

fn dialog_open(app: &App, cx: &mut TestAppContext) -> bool {
    cx.update_window(app.window, |_, window, cx| {
        gpui_kit::component::WindowExt::has_active_dialog(window, cx)
    })
    .expect("window")
}

fn create_and_open_note(app: &App, cx: &mut TestAppContext, title: &str) -> ottero_core::NoteId {
    let id = cx.update(|cx| {
        app.shell.read(cx).library().clone().update(cx, |lib, cx| {
            let id = lib.create_note(title, cx).expect("create");
            lib.edit_note(id, title, "body", cx).expect("edit");
            id
        })
    });
    cx.update_window(app.window, |_, window, cx| {
        app.shell
            .update(cx, |shell, cx| shell.open_note(id, window, cx));
    })
    .expect("window");
    cx.run_until_parked();
    id
}

fn note_state(app: &App, cx: &mut TestAppContext, id: ottero_core::NoteId) -> Option<bool> {
    cx.read(|cx| {
        app.shell
            .read(cx)
            .library()
            .read(cx)
            .note(id)
            .expect("load")
            .map(|n| n.is_trashed())
    })
}

#[gpui_kit::test]
fn toasts_never_cover_the_toolbar(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    create_and_open_note(&app, cx, "Groceries");
    click(&app, cx, "note-more");
    click_label(&app, cx, "Move to trash");
    cx.update_window(app.window, |_, window, cx| {
        window.render_frame(cx);
        let toast = window.find("undo").bounds();
        let height = window.bounds().size.height;
        assert!(
            toast.top() > height * 0.5,
            "toast sits in the lower half: {toast:?}"
        );
    })
    .expect("window");
}

#[gpui_kit::test]
fn trash_is_immediate_and_undo_restores(cx: &mut TestAppContext) {
    let app = launch(cx);
    // Toasts fade in on a wall-clock animation; reduced motion shows them at once.
    cx.update(|cx| cx.set_reduce_motion(true));
    let id = create_and_open_note(&app, cx, "Groceries");

    click(&app, cx, "note-more");
    click_label(&app, cx, "Move to trash");
    assert!(
        !dialog_open(&app, cx),
        "reversible actions must not ask for confirmation"
    );
    assert_eq!(note_state(&app, cx, id), Some(true));

    click(&app, cx, "undo");
    assert_eq!(
        note_state(&app, cx, id),
        Some(false),
        "Undo restores the note"
    );
}

#[gpui_kit::test]
fn delete_forever_requires_confirmation(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let id = create_and_open_note(&app, cx, "Old draft");
    click(&app, cx, "note-more");
    click_label(&app, cx, "Move to trash");
    assert_eq!(
        note_state(&app, cx, id),
        Some(true),
        "moved to trash from the … menu"
    );
    // Open the trashed note (read-only view shows Restore / Delete forever).
    cx.update_window(app.window, |_, window, cx| {
        app.shell
            .update(cx, |shell, cx| shell.open_note(id, window, cx));
    })
    .expect("window");
    cx.run_until_parked();

    click(&app, cx, "note-delete");
    assert!(dialog_open(&app, cx), "permanent delete asks first");
    press(&app, cx, "escape");
    assert!(!dialog_open(&app, cx));
    assert_eq!(
        note_state(&app, cx, id),
        Some(true),
        "Escape keeps the note"
    );

    click(&app, cx, "note-delete");
    press(&app, cx, "enter");
    assert!(!dialog_open(&app, cx));
    assert_eq!(note_state(&app, cx, id), None, "confirming deletes it");
}

fn all_note_titles(app: &App, cx: &mut TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        app.shell
            .read(cx)
            .library()
            .read(cx)
            .notes(false)
            .expect("notes")
            .into_iter()
            .map(|n| n.title)
            .collect()
    })
}

#[gpui_kit::test]
fn title_input_stops_at_the_length_limit(cx: &mut TestAppContext) {
    let app = launch(cx);
    dispatch(&app, cx, NewNote);
    type_text(&app, cx, &"x".repeat(ottero_core::MAX_TITLE_CHARS + 20));
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    let titles = all_note_titles(&app, cx);
    assert!(
        titles
            .iter()
            .any(|t| t.chars().count() == ottero_core::MAX_TITLE_CHARS),
        "long title is capped and saved: {:?}",
        titles.iter().map(|t| t.len()).collect::<Vec<_>>()
    );
}

#[gpui_kit::test]
fn failed_save_keeps_the_editor_open(cx: &mut TestAppContext) {
    let app = launch(cx);
    let first = create_and_open_note(&app, cx, "First");
    let second = cx.update(|cx| {
        app.shell
            .read(cx)
            .library()
            .clone()
            .update(cx, |lib, cx| lib.create_note("Second", cx))
            .expect("create")
    });
    // Type into the open note, then make its save fail by deleting it underneath.
    cx.update_window(app.window, |_, window, cx| {
        app.shell
            .read(cx)
            .notes()
            .clone()
            .update(cx, |notes, cx| notes.focus_body(window, cx));
    })
    .expect("window");
    cx.run_until_parked();
    type_text(&app, cx, " unsaved words");
    cx.update(|cx| {
        app.shell
            .read(cx)
            .library()
            .clone()
            .update(cx, |lib, cx| lib.delete_note_permanently(first, cx))
            .expect("delete");
    });
    // An optional update must refuse exit for the same failure, preserving the
    // draft before attempting any installer operation.
    let can_restart = cx
        .update_window(app.window, |_, window, cx| {
            app.shell
                .update(cx, |shell, cx| shell.prepare_update_restart(window, cx))
        })
        .expect("window");
    assert!(!can_restart, "a failed save must block an update restart");
    // Switching must refuse rather than discard the typing.
    cx.update_window(app.window, |_, window, cx| {
        app.shell
            .update(cx, |shell, cx| shell.open_note(second, window, cx));
    })
    .expect("window");
    cx.run_until_parked();
    let body = cx.read(|cx| app.shell.read(cx).notes().read(cx).body_text(cx));
    assert!(
        body.contains("unsaved words"),
        "editor kept the typing: {body:?}"
    );
}

#[gpui_kit::test]
fn update_restart_flushes_a_pending_note_before_autosave(cx: &mut TestAppContext) {
    let app = launch(cx);
    let id = create_and_open_note(&app, cx, "Update draft");
    cx.update_window(app.window, |_, window, cx| {
        app.shell
            .read(cx)
            .notes()
            .clone()
            .update(cx, |notes, cx| notes.focus_body(window, cx));
    })
    .expect("window");
    cx.run_until_parked();
    type_text(&app, cx, "Keep this pending edit");
    let can_restart = cx
        .update_window(app.window, |_, window, cx| {
            app.shell
                .update(cx, |shell, cx| shell.prepare_update_restart(window, cx))
        })
        .expect("window");
    assert!(can_restart);
    let saved = cx.read(|cx| {
        app.shell
            .read(cx)
            .library()
            .read(cx)
            .note(id)
            .expect("load")
            .expect("note")
            .body()
            .to_owned()
    });
    assert!(saved.contains("Keep this pending edit"));
}

#[gpui_kit::test]
fn release_history_opens_from_about_and_returns_by_button_or_escape(cx: &mut TestAppContext) {
    let app = launch(cx);
    dispatch(&app, cx, GoSettings);
    cx.update_window(app.window, |_, w, cx| {
        w.within("settings-sidebar").click("0-7", cx);
    })
    .expect("About");
    cx.run_until_parked();
    for light in [false, true] {
        if light {
            dispatch(&app, cx, UseLightTheme);
        } else {
            dispatch(&app, cx, UseDarkTheme);
        }
        cx.update_window(app.window, |_, w, cx| {
            w.click("set-whats-new", cx);
        })
        .expect("open release notes");
        cx.run_until_parked();
        cx.update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            let texts = gpui_kit::base::test_support::texts(w);
            assert!(texts.iter().any(|t| t.text() == "What’s new"));
            assert!(texts.iter().any(|t| t.text() == "Calmfold is now Ottero"));
            assert!(texts.iter().any(|t| t.text() == "Installed"));
            w.click("release-0.1.1", cx);
        })
        .expect("older release");
        cx.run_until_parked();
        cx.update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            let texts = gpui_kit::base::test_support::texts(w);
            assert!(
                texts
                    .iter()
                    .any(|t| t.text() == "Small details, smoother days")
            );
            assert!(!texts.iter().any(|t| t.text() == "Calmfold is now Ottero"));
            w.click("release-0.1.3", cx);
        })
        .expect("current release");
        cx.run_until_parked();
        cx.update_window(app.window, |_, w, cx| {
            w.click("whats-new-back", cx);
        })
        .expect("read and return");
        cx.run_until_parked();
        dispatch(&app, cx, ottero_desktop::actions::ShowChangelog);
        press(&app, cx, "escape");
        cx.run_until_parked();
        cx.update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            assert!(
                !gpui_kit::base::test_support::texts(w)
                    .iter()
                    .any(|t| t.text() == "Calmfold is now Ottero")
            );
            w.click("set-whats-new", cx);
        })
        .expect("Escape restores About");
        cx.run_until_parked();
        cx.update_window(app.window, |_, w, cx| {
            w.click("whats-new-back", cx);
        })
        .expect("return again");
        cx.run_until_parked();
    }
    dispatch(&app, cx, ottero_desktop::actions::ShowChangelog);
    cx.simulate_window_resize(
        app.window,
        gpui_kit::size(gpui_kit::px(800.), gpui_kit::px(650.)),
    );
    cx.run_until_parked();
    click(&app, cx, "release-version-picker");
    click_label(&app, cx, "Version 0.1.0");
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert!(
            gpui_kit::base::test_support::texts(w)
                .iter()
                .any(|t| t.text() == "A quieter daily companion")
        );
    })
    .expect("narrow release selection");
}

#[gpui_kit::test]
fn update_status_refreshes_in_settings_without_navigation(cx: &mut TestAppContext) {
    use gpui_kit::BorrowAppContext as _;
    use ottero_desktop::updates::{Status, Updates};
    let app = launch(cx);
    dispatch(&app, cx, GoSettings);
    cx.update_window(app.window, |_, w, cx| {
        w.within("settings-sidebar").click("0-7", cx);
    })
    .expect("About");
    cx.run_until_parked();
    for theme in [false, true] {
        if theme {
            dispatch(&app, cx, UseLightTheme);
        } else {
            dispatch(&app, cx, UseDarkTheme);
        }
        for (status, expected) in [
            (
                Status::Downloading {
                    version: "0.1.2".into(),
                },
                "Downloading 0.1.2…",
            ),
            (Status::UpToDate, "Up to date"),
        ] {
            cx.update(|cx| cx.update_global::<Updates, _>(|updates, _| updates.status = status));
            cx.run_until_parked();
            cx.update_window(app.window, |_, w, cx| {
                w.render_frame(cx);
                assert!(
                    gpui_kit::base::test_support::texts(w)
                        .iter()
                        .any(|t| t.text() == expected),
                    "live update status: {expected}; rendered: {:?}",
                    gpui_kit::base::test_support::texts(w)
                        .iter()
                        .map(|t| t.text().to_owned())
                        .collect::<Vec<_>>()
                );
            })
            .expect("status text");
        }
    }
}

#[gpui_kit::test]
fn palette_new_note_leaves_focus_in_the_title(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    dispatch(&app, cx, OpenCommandPalette);
    type_text(&app, cx, "New note");
    press(&app, cx, "enter");
    cx.run_until_parked();
    assert_eq!(route(&app, cx), Route::Notes);
    type_text(&app, cx, "From palette");
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    assert!(
        all_note_titles(&app, cx).contains(&"From palette".to_owned()),
        "{:?}",
        all_note_titles(&app, cx)
    );
}

#[gpui_kit::test]
fn typing_double_bracket_suggests_and_completes_note_titles(cx: &mut TestAppContext) {
    let app = launch(cx);
    create_and_open_note(&app, cx, "Service log");
    dispatch(&app, cx, NewNote);
    type_text(&app, cx, "Car");
    press(&app, cx, "enter");
    type_text(&app, cx, "See [[serv");

    let editor = cx.read(|cx| app.shell.read(cx).notes().read(cx).note_editor().clone());
    let labels = |cx: &mut TestAppContext| {
        cx.read(|cx| editor.read(cx).link_suggestions().map(<[String]>::to_vec))
    };
    assert_eq!(
        labels(cx),
        Some(vec!["Service log".to_owned()]),
        "the menu opens inside [["
    );

    press(&app, cx, "enter");
    let body = cx.read(|cx| app.shell.read(cx).notes().read(cx).body_text(cx));
    assert_eq!(
        body, "See [[Service log]]",
        "Enter accepts instead of a new line"
    );

    // Outside a link, nothing is suggested.
    type_text(&app, cx, " and more");
    assert_eq!(labels(cx), None);

    // Escape closes the menu and keeps the typed text.
    type_text(&app, cx, " [[Ser");
    assert!(labels(cx).is_some());
    press(&app, cx, "escape");
    assert_eq!(labels(cx), None);
    let body = cx.read(|cx| app.shell.read(cx).notes().read(cx).body_text(cx));
    assert_eq!(body, "See [[Service log]] and more [[Ser");
}

#[gpui_kit::test]
fn following_a_link_opens_existing_or_creates_missing_notes(cx: &mut TestAppContext) {
    let app = launch(cx);
    let car = create_and_open_note(&app, cx, "My Car");
    let follow = |target: &'static str, cx: &mut TestAppContext| {
        cx.update_window(app.window, |_, window, cx| {
            app.shell
                .read(cx)
                .notes()
                .clone()
                .update(cx, |notes, cx| notes.follow_link(target, window, cx));
        })
        .expect("window");
        cx.run_until_parked();
        cx.read(|cx| app.shell.read(cx).notes().read(cx).open_note_id())
    };
    assert_eq!(
        follow("my  car", cx),
        Some(car),
        "matches case- and space-insensitively"
    );

    let created = follow("Trip ideas", cx).expect("opened");
    assert_ne!(created, car);
    assert!(
        all_note_titles(&app, cx).contains(&"Trip ideas".to_owned()),
        "missing target is created"
    );
    assert_eq!(
        follow("trip IDEAS", cx),
        Some(created),
        "no duplicate on second click"
    );
}

#[gpui_kit::test]
fn task_detail_edits_status_due_notes_and_title(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let id = cx.update(|cx| {
        app.shell
            .read(cx)
            .library()
            .clone()
            .update(cx, |lib, cx| lib.create_task("Pay rent", None, cx))
            .expect("create")
    });
    dispatch(&app, cx, GoTasks);
    cx.update_window(app.window, |_, window, cx| {
        app.shell
            .read(cx)
            .tasks()
            .clone()
            .update(cx, |t, cx| t.set_tab(TaskTab::All, cx));
        window.render_frame(cx);
    })
    .expect("window");
    cx.run_until_parked();

    let task_id = format!("task-{id}-open");
    let open_id: &'static str = Box::leak(task_id.into_boxed_str());
    click(&app, cx, open_id);
    let sheet_open = |cx: &mut TestAppContext| {
        cx.update_window(app.window, |_, window, cx| {
            gpui_kit::component::WindowExt::has_active_sheet(window, cx)
        })
        .expect("window")
    };
    assert!(sheet_open(cx), "clicking a task opens its detail panel");
    let can_restart = cx
        .update_window(app.window, |_, window, cx| {
            app.shell
                .update(cx, |shell, cx| shell.prepare_update_restart(window, cx))
        })
        .expect("window");
    assert!(!can_restart, "an update cannot close an open task editor");
    assert!(sheet_open(cx), "update refusal keeps the editor open");

    let task = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            app.shell
                .read(cx)
                .library()
                .read(cx)
                .tasks_by_id(id)
                .expect("task")
        })
    };

    click(&app, cx, "detail-start");
    assert_eq!(task(cx).status(), TaskStatus::InProgress);
    click(&app, cx, "detail-complete");
    assert_eq!(task(cx).status(), TaskStatus::Done);
    click(&app, cx, "detail-reopen");
    assert_eq!(task(cx).status(), TaskStatus::Todo);

    click(&app, cx, "due-tomorrow");
    assert_eq!(task(cx).due(), ottero_core::clock::today().tomorrow().ok());
    click(&app, cx, "due-none");
    assert_eq!(task(cx).due(), None);

    // Enter in the title moves to the notes field.
    click(&app, cx, "detail-title");
    press(&app, cx, "enter");
    type_text(&app, cx, "Transfer by the 1st");
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    assert_eq!(task(cx).notes(), "Transfer by the 1st");

    // A title edit is saved even if the panel closes before the autosave timer.
    click(&app, cx, "detail-title");
    type_text(&app, cx, " and water bill");
    cx.update_window(app.window, |_, window, cx| {
        gpui_kit::component::WindowExt::close_sheet(window, cx)
    })
    .expect("window");
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    assert!(!sheet_open(cx));
    assert_eq!(task(cx).title(), "Pay rent and water bill");
}

/// Clicks the element whose accessibility label is exactly `label`.
fn click_label(app: &App, cx: &mut TestAppContext, label: &str) {
    use gpui_kit::test::{TestEventExt, TestQueryExt};
    cx.update_window(app.window, |_, window, cx| {
        window.render_frame(cx);
        let center = window.find_by_label(label).bounds().center();
        window.pointer_down(center, gpui_kit::MouseButton::Left, cx);
        window.pointer_up(center, gpui_kit::MouseButton::Left, cx);
    })
    .expect("window");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn task_menu_sets_priority_and_due_through_submenus(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let id = cx.update(|cx| {
        app.shell
            .read(cx)
            .library()
            .clone()
            .update(cx, |lib, cx| lib.create_task("Call bank", None, cx))
            .expect("create")
    });
    dispatch(&app, cx, GoTasks);
    cx.update_window(app.window, |_, _, cx| {
        app.shell
            .read(cx)
            .tasks()
            .clone()
            .update(cx, |t, cx| t.set_tab(TaskTab::All, cx));
    })
    .expect("window");
    cx.run_until_parked();
    let menu: &'static str = Box::leak(format!("task-{id}-menu").into_boxed_str());
    let task = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            app.shell
                .read(cx)
                .library()
                .read(cx)
                .tasks_by_id(id)
                .expect("task")
        })
    };

    click(&app, cx, menu);
    click_label(&app, cx, "Priority");
    click_label(&app, cx, "High");
    assert_eq!(task(cx).priority(), ottero_core::Priority::High);

    // The submenu label now shows the current value.
    click(&app, cx, menu);
    click_label(&app, cx, "Due date");
    click_label(&app, cx, "Tomorrow");
    assert_eq!(task(cx).due(), ottero_core::clock::today().tomorrow().ok());

    click(&app, cx, menu);
    click_label(&app, cx, "Start");
    assert_eq!(task(cx).status(), TaskStatus::InProgress);
}

#[gpui_kit::test]
fn note_list_panel_collapses_resizes_and_persists(cx: &mut TestAppContext) {
    use ottero_desktop::actions::ToggleListPanel;
    use ottero_desktop::ui_state::{LayoutState, NOTES_LIST, layout_file};
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let panel =
        |cx: &mut TestAppContext| cx.read(|cx| app.shell.read(cx).layout(cx).panel(NOTES_LIST));
    assert!(!panel(cx).collapsed);
    assert_eq!(panel(cx).width, NOTES_LIST.default);

    // The shortcut also switches to Notes, where the list lives.
    dispatch(&app, cx, ToggleListPanel);
    assert_eq!(route(&app, cx), Route::Notes);
    assert!(panel(cx).collapsed);

    // The title-bar toggle stays in place and reopens it.
    click(&app, cx, "toggle-list-panel");
    assert!(!panel(cx).collapsed);

    // Drag the panel edge 60 px to the right.
    use gpui_kit::test::TestEventExt;
    cx.update_window(app.window, |_, window, cx| {
        window.render_frame(cx);
        let edge = window.find("notes-list-edge").bounds().center();
        let to = edge + gpui_kit::point(gpui_kit::px(60.), gpui_kit::px(0.));
        window.pointer_down(edge, gpui_kit::MouseButton::Left, cx);
        // Like a real mouse: several moves (the first one only starts the drag).
        for step in 1..=6 {
            let p = edge + gpui_kit::point(gpui_kit::px(10. * step as f32), gpui_kit::px(0.));
            window.pointer_move(p, Some(gpui_kit::MouseButton::Left), cx);
        }
        window.pointer_up(to, gpui_kit::MouseButton::Left, cx);
    })
    .expect("window");
    cx.run_until_parked();
    let width = panel(cx).width;
    assert!(
        (width - (NOTES_LIST.default + 60.)).abs() < 0.5,
        "moves exactly with the pointer, no jump: {width}"
    );

    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    let saved = LayoutState::load(&layout_file(&app.paths.data_dir)).panel(NOTES_LIST);
    assert!((saved.width - width).abs() < 0.5, "width saved to disk");
}

#[gpui_kit::test]
fn command_k_toggles_the_palette_and_never_stacks(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let dialogs = |cx: &mut TestAppContext| {
        cx.update_window(app.window, |_, window, cx| {
            gpui_kit::component::WindowExt::has_active_dialog(window, cx)
        })
        .expect("window")
    };
    let key = if cfg!(target_os = "macos") {
        "cmd-k"
    } else {
        "ctrl-k"
    };

    press(&app, cx, key);
    assert!(dialogs(cx), "first press opens");
    press(&app, cx, key);
    assert!(!dialogs(cx), "second press closes instead of stacking");
    press(&app, cx, key);
    press(&app, cx, key);
    press(&app, cx, key);
    assert!(dialogs(cx), "odd number of presses leaves exactly one open");
    press(&app, cx, "escape");
    assert!(!dialogs(cx), "one Escape closes it: there was only one");

    // The sidebar entry opens the same palette.
    click(&app, cx, "open-palette");
    assert!(dialogs(cx));
}

// ---- Notes, Tasks and Today: the "finish" batch ----

fn secondary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn library(app: &App, cx: &mut TestAppContext) -> Entity<ottero_desktop::library::Library> {
    cx.read(|cx| app.shell.read(cx).library().clone())
}

fn task_by_id(app: &App, cx: &mut TestAppContext, id: ottero_core::TaskId) -> ottero_core::Task {
    cx.read(|cx| {
        app.shell
            .read(cx)
            .library()
            .read(cx)
            .tasks_by_id(id)
            .expect("task")
    })
}

fn open_tasks(app: &App, cx: &mut TestAppContext) -> Vec<ottero_core::Task> {
    cx.read(|cx| {
        app.shell
            .read(cx)
            .library()
            .read(cx)
            .tasks(TaskFilter::Open)
            .expect("tasks")
    })
}

#[gpui_kit::test]
fn quick_add_reads_date_priority_and_project_from_the_text(cx: &mut TestAppContext) {
    let app = launch(cx);
    let lib = library(&app, cx);
    let home = cx.update(|cx| {
        lib.update(cx, |l, cx| l.create_project("Home", cx))
            .expect("project")
    });

    dispatch(&app, cx, NewTask);
    type_text(&app, cx, "Plan Monday standup tomorrow !high #Home");
    press(&app, cx, "enter");
    // An unknown project stays in the title instead of being guessed.
    type_text(&app, cx, "Buy paint #Garage");
    press(&app, cx, "enter");

    let tasks = open_tasks(&app, cx);
    let plan = tasks
        .iter()
        .find(|t| t.title() == "Plan Monday standup")
        .expect("dates are read only from the end of the text");
    assert_eq!(plan.due(), ottero_core::clock::today().tomorrow().ok());
    assert_eq!(plan.priority(), ottero_core::Priority::High);
    assert_eq!(plan.project(), Some(home));

    let paint = tasks
        .iter()
        .find(|t| t.title().starts_with("Buy paint"))
        .expect("paint");
    assert_eq!(paint.title(), "Buy paint #Garage");
    assert_eq!(paint.project(), None);
    assert_eq!(
        paint.due(),
        Some(ottero_core::clock::today()),
        "no date in the text: the Today tab's default applies"
    );
}

#[gpui_kit::test]
fn completing_a_repeating_task_schedules_the_next_one(cx: &mut TestAppContext) {
    let app = launch(cx);
    dispatch(&app, cx, NewTask);
    type_text(&app, cx, "Water plants every day");
    press(&app, cx, "enter");

    let today = ottero_core::clock::today();
    let first = open_tasks(&app, cx);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].title(), "Water plants");
    assert_eq!(
        first[0].recurrence(),
        Some(ottero_core::Recurrence::Days(1))
    );
    assert_eq!(first[0].due(), Some(today));

    let id = first[0].id();
    let lib = library(&app, cx);
    cx.update(|cx| {
        lib.update(cx, |l, cx| l.toggle_task(id, cx))
            .expect("complete")
    });

    assert_eq!(task_by_id(&app, cx, id).status(), TaskStatus::Done);
    let next = open_tasks(&app, cx);
    assert_eq!(next.len(), 1, "exactly one new instance");
    assert_ne!(next[0].id(), id);
    assert_eq!(next[0].title(), "Water plants");
    assert_eq!(next[0].due(), today.tomorrow().ok());
    assert_eq!(next[0].recurrence(), Some(ottero_core::Recurrence::Days(1)));
}

#[gpui_kit::test]
fn task_detail_adds_checks_and_removes_steps(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let lib = library(&app, cx);
    let id = cx.update(|cx| {
        lib.update(cx, |l, cx| l.create_task("Move house", None, cx))
            .expect("create")
    });
    dispatch(&app, cx, GoTasks);
    cx.update_window(app.window, |_, _, cx| {
        app.shell
            .read(cx)
            .tasks()
            .clone()
            .update(cx, |t, cx| t.set_tab(TaskTab::All, cx));
    })
    .expect("window");
    cx.run_until_parked();
    click(&app, cx, leak(format!("task-{id}-open")));

    click(&app, cx, "new-step");
    type_text(&app, cx, "Book van");
    press(&app, cx, "enter");
    type_text(&app, cx, "Pack books");
    press(&app, cx, "enter");

    let steps = |cx: &mut TestAppContext| cx.read(|cx| lib.read(cx).subtasks(id));
    let progress =
        |cx: &mut TestAppContext| cx.read(|cx| lib.read(cx).subtask_progress().get(&id).copied());
    let titles: Vec<String> = steps(cx).iter().map(|s| s.title().to_owned()).collect();
    assert_eq!(
        titles,
        ["Book van", "Pack books"],
        "Enter adds a step and clears the field"
    );
    assert_eq!(progress(cx).map(|p| (p.done, p.total)), Some((0, 2)));
    assert!(
        open_tasks(&app, cx).iter().all(|t| t.parent().is_none()),
        "steps don't show up as top-level tasks"
    );

    let van = steps(cx)[0].id();
    click(&app, cx, leak(format!("step-{van}")));
    assert_eq!(progress(cx).map(|p| (p.done, p.total)), Some((1, 2)));

    click(&app, cx, leak(format!("step-{van}-remove")));
    assert_eq!(steps(cx).len(), 1);
    assert_eq!(progress(cx).map(|p| (p.done, p.total)), Some((0, 1)));
}

#[gpui_kit::test]
fn projects_are_created_shown_renamed_and_deleted_without_losing_tasks(cx: &mut TestAppContext) {
    use ottero_desktop::actions::NewProject;
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));

    dispatch(&app, cx, NewProject);
    assert!(dialog_open(&app, cx), "New project asks for a name");
    type_text(&app, cx, "Garden");
    press(&app, cx, "enter");
    assert!(!dialog_open(&app, cx));

    let lib = library(&app, cx);
    let projects = cx.read(|cx| lib.read(cx).projects());
    assert_eq!(projects.len(), 1);
    let garden = projects[0].0.id();
    assert_eq!(projects[0].0.name(), "Garden");
    assert_eq!(route(&app, cx), Route::Tasks);
    assert_eq!(
        cx.read(|cx| app.shell.read(cx).tasks().read(cx).project()),
        Some(garden),
        "the new project opens"
    );

    // Quick add inside a project files the task there, with no default date.
    click(&app, cx, "task-input");
    type_text(&app, cx, "Plant tulips");
    press(&app, cx, "enter");
    let tulips = open_tasks(&app, cx)
        .into_iter()
        .find(|t| t.title() == "Plant tulips")
        .expect("task");
    assert_eq!(tulips.project(), Some(garden));
    assert_eq!(tulips.due(), None);

    click(&app, cx, "project-more");
    click_label(&app, cx, "Rename…");
    assert!(dialog_open(&app, cx));
    press(&app, cx, &secondary("a"));
    type_text(&app, cx, "Backyard");
    press(&app, cx, "enter");
    assert_eq!(
        cx.read(|cx| lib.read(cx).project(garden))
            .map(|p| p.name().to_owned()),
        Some("Backyard".to_owned())
    );

    click(&app, cx, "project-more");
    click_label(&app, cx, "Delete project…");
    assert!(dialog_open(&app, cx), "deleting a project asks first");
    press(&app, cx, "enter");
    assert!(cx.read(|cx| lib.read(cx).projects()).is_empty());
    let kept = task_by_id(&app, cx, tulips.id());
    assert_eq!(kept.project(), None, "its tasks stay, unfiled");
    assert!(!kept.is_trashed());
    assert_eq!(
        cx.read(|cx| app.shell.read(cx).tasks().read(cx).project()),
        None
    );
}

#[gpui_kit::test]
fn daily_note_opens_todays_note_and_never_duplicates_it(cx: &mut TestAppContext) {
    use ottero_desktop::actions::OpenDailyNote;
    let app = launch(cx);
    dispatch(&app, cx, OpenDailyNote);
    assert_eq!(route(&app, cx), Route::Notes);
    let title = ottero_desktop::library::daily_note_title(ottero_core::clock::today());
    let first = cx.read(|cx| app.shell.read(cx).notes().read(cx).open_note_id());
    assert!(first.is_some());
    assert_eq!(
        cx.read(|cx| app.shell.read(cx).notes().read(cx).open_note_title(cx)),
        Some(title.clone())
    );

    dispatch(&app, cx, GoToday);
    dispatch(&app, cx, OpenDailyNote);
    assert_eq!(
        cx.read(|cx| app.shell.read(cx).notes().read(cx).open_note_id()),
        first
    );
    assert_eq!(
        all_note_titles(&app, cx)
            .iter()
            .filter(|t| **t == title)
            .count(),
        1
    );
}

#[gpui_kit::test]
fn tag_filter_shows_only_tagged_notes(cx: &mut TestAppContext) {
    let app = launch(cx);
    let lib = library(&app, cx);
    let (car, recipe) = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let car = l.create_note("Car", cx).expect("create");
            l.edit_note(car, "Car", "Tyres next month #car #todo", cx)
                .expect("edit");
            let recipe = l.create_note("Soup", cx).expect("create");
            l.edit_note(recipe, "Soup", "Lentils #cooking", cx)
                .expect("edit");
            (car, recipe)
        })
    });
    let tags: Vec<String> = cx
        .read(|cx| lib.read(cx).note_tags())
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    assert!(
        tags.contains(&"car".to_owned()) && tags.contains(&"cooking".to_owned()),
        "{tags:?}"
    );

    dispatch(&app, cx, GoNotes);
    let shown = |id: ottero_core::NoteId, cx: &mut TestAppContext| {
        cx.update_window(app.window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(leak(format!("note-{id}"))).is_some()
        })
        .expect("window")
    };
    assert!(shown(car, cx) && shown(recipe, cx));

    cx.update_window(app.window, |_, window, cx| {
        app.shell.read(cx).notes().clone().update(cx, |n, cx| {
            n.set_tag_filter(Some("#Car".into()), window, cx);
        });
    })
    .expect("window");
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| app
            .shell
            .read(cx)
            .notes()
            .read(cx)
            .tag_filter()
            .map(str::to_owned)),
        Some("car".to_owned()),
        "tags are normalized"
    );
    assert!(shown(car, cx));
    assert!(!shown(recipe, cx), "untagged notes are hidden");

    cx.update_window(app.window, |_, window, cx| {
        app.shell
            .read(cx)
            .notes()
            .clone()
            .update(cx, |n, cx| n.set_tag_filter(None, window, cx));
    })
    .expect("window");
    cx.run_until_parked();
    assert!(
        shown(recipe, cx),
        "clearing the filter shows everything again"
    );
}

#[gpui_kit::test]
fn unlinked_mentions_can_be_turned_into_links(cx: &mut TestAppContext) {
    let app = launch(cx);
    let lib = library(&app, cx);
    let trip = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let trip = l.create_note("Trip", cx).expect("create");
            let packing = l.create_note("Packing", cx).expect("create");
            l.edit_note(packing, "Packing", "Things for the trip: tent", cx)
                .expect("edit");
            trip
        })
    });
    let mentions = cx.read(|cx| lib.read(cx).unlinked_mentions(trip, "Trip"));
    assert_eq!(mentions.len(), 1);
    let source = mentions[0].id;

    let linked = cx.update(|cx| {
        lib.update(cx, |l, cx| l.link_mention(source, "Trip", cx))
            .expect("link")
    });
    assert!(linked);
    let body = cx.read(|cx| {
        lib.read(cx)
            .note(source)
            .expect("load")
            .expect("note")
            .body()
            .to_owned()
    });
    assert_eq!(
        body, "Things for the [[trip]]: tent",
        "keeps the original casing"
    );
    assert!(
        cx.read(|cx| lib.read(cx).unlinked_mentions(trip, "Trip"))
            .is_empty()
    );
    assert_eq!(
        cx.read(|cx| lib.read(cx).backlinks("Trip"))
            .expect("backlinks")
            .len(),
        1
    );
}

#[gpui_kit::test]
fn format_shortcuts_edit_the_selection_as_markdown(cx: &mut TestAppContext) {
    let app = launch(cx);
    create_and_open_note(&app, cx, "Formatting");
    cx.update_window(app.window, |_, window, cx| {
        app.shell
            .read(cx)
            .notes()
            .clone()
            .update(cx, |n, cx| n.focus_body(window, cx));
    })
    .expect("window");
    cx.run_until_parked();
    let body =
        |cx: &mut TestAppContext| cx.read(|cx| app.shell.read(cx).notes().read(cx).body_text(cx));

    press(&app, cx, &secondary("a"));
    press(&app, cx, &secondary("b"));
    assert_eq!(body(cx), "**body**");
    // The selection stays on the text, so the same shortcut toggles it off.
    press(&app, cx, &secondary("b"));
    assert_eq!(body(cx), "body");

    press(&app, cx, &secondary("alt-1"));
    assert_eq!(body(cx), "# body");

    // Shortcuts only act inside the editor: in the title they do nothing.
    cx.update_window(app.window, |_, window, cx| {
        app.shell.read(cx).notes().clone().update(cx, |n, cx| {
            n.focus_search(window, cx);
        });
    })
    .expect("window");
    cx.run_until_parked();
    press(&app, cx, &secondary("b"));
    assert_eq!(body(cx), "# body");
}

#[gpui_kit::test]
fn long_notes_scroll_the_page_not_the_editor(cx: &mut TestAppContext) {
    let app = launch(cx);
    let lib = library(&app, cx);
    let long: String = (1..=80).map(|i| format!("Line {i}\n")).collect();
    let id = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let id = l.create_note("Long", cx).expect("create");
            l.edit_note(id, "Long", &long, cx).expect("edit");
            id
        })
    });
    cx.update_window(app.window, |_, window, cx| {
        app.shell.update(cx, |s, cx| s.open_note(id, window, cx));
    })
    .expect("window");
    cx.run_until_parked();

    // Row counts are known after layout.
    let (body, tail, page, window_height) = cx
        .update_window(app.window, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            (
                window.find("note-body").bounds(),
                window.find("note-body-tail").bounds(),
                window.find("note-scroll").bounds(),
                window.bounds().size.height,
            )
        })
        .expect("window");
    let font = cx.read(|cx| Theme::global(cx).font_size);
    let text_height = body.size.height - tail.size.height;
    let rows_shown = text_height / (font * zorite_editor::LINE_HEIGHT_RATIO);
    assert!(
        rows_shown >= 80.,
        "editor is as tall as its 80 lines, so it never scrolls inside: {rows_shown}"
    );
    assert!(
        body.size.height > page.size.height && page.size.height <= window_height,
        "the page is the one scroll area: editor {body:?}, page {page:?}"
    );
}

#[gpui_kit::test]
fn today_shows_the_week_and_the_next_seven_days(cx: &mut TestAppContext) {
    let app = launch(cx);
    let lib = library(&app, cx);
    let today = ottero_core::clock::today();
    let in_three = today.checked_add(jiff::Span::new().days(3)).expect("date");
    let in_twenty = today.checked_add(jiff::Span::new().days(20)).expect("date");
    let (soon, later) = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let soon = l
                .create_task("Dentist", Some(in_three), cx)
                .expect("create");
            let later = l
                .create_task("Renew passport", Some(in_twenty), cx)
                .expect("create");
            let done = l.create_task("Pay rent", Some(today), cx).expect("create");
            l.toggle_task(done, cx).expect("complete");
            (soon, later)
        })
    });

    let week = cx.read(|cx| lib.read(cx).completions_by_day(7));
    assert_eq!(week.len(), 7);
    assert_eq!(
        week.last(),
        Some(&(today, 1)),
        "today's completion is counted"
    );

    dispatch(&app, cx, GoToday);
    cx.update_window(app.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("week-chart").is_some(),
            "week chart renders"
        );
        assert!(
            window.try_find(leak(format!("upcoming-{soon}"))).is_some(),
            "due in 3 days is listed"
        );
        assert!(
            window.try_find(leak(format!("upcoming-{later}"))).is_none(),
            "20 days out is not"
        );
    })
    .expect("window");
}

// ---- Live-preview editor (zorite-editor, ADR 0004) ----

fn note_editor(
    app: &App,
    cx: &mut TestAppContext,
) -> Entity<ottero_desktop::views::note_editor::NoteEditor> {
    cx.read(|cx| app.shell.read(cx).notes().read(cx).note_editor().clone())
}

fn open_note_with(
    app: &App,
    cx: &mut TestAppContext,
    title: &str,
    body: &str,
) -> ottero_core::NoteId {
    let lib = library(app, cx);
    let id = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let id = l.create_note(title, cx).expect("create");
            l.edit_note(id, title, body, cx).expect("edit");
            id
        })
    });
    cx.update_window(app.window, |_, window, cx| {
        app.shell.update(cx, |s, cx| s.open_note(id, window, cx));
        window.render_frame(cx);
    })
    .expect("window");
    cx.run_until_parked();
    id
}

#[gpui_kit::test]
fn opening_and_saving_never_changes_a_notes_markdown(cx: &mut TestAppContext) {
    let app = launch(cx);
    // Things block editors are known to rewrite: blank lines, loose lists,
    // trailing spaces, tables, HTML, math, CRLF, no final newline.
    let samples = [
        "",
        "Plain line",
        "# Title\n\n\n\nThree blank lines above.\n",
        "- a\n\n- loose list\n  - nested\n\n1) other\n2) numbering\n",
        "Hard break with two spaces  \nnext line\t\ttabs",
        "| A | B |\n|:--|--:|\n| 1 | 2 |\n\n<!-- table:striped -->\n",
        "**bold** _it_ ~~s~~ ==hi== `code` <u>u</u> <span style=\"color:#f00\">red</span>",
        "```rust\nfn main() {}\n```\n~~~\nraw\n~~~",
        "> [!NOTE]\n> callout\n\n---\n\n[^1]: foot",
        "$$\nx^2 + y^2\n$$\n\nInline $a+b$ math.",
        "Windows\r\nline endings\r\n",
        "Emoji 😀, accents é, RTL שלום, [[Link|alias]] #tag/nested",
    ];
    let editor = note_editor(&app, cx);
    for sample in samples {
        cx.update(|cx| editor.update(cx, |e, cx| e.set_note(None, sample, cx)));
        cx.update_window(app.window, |_, window, cx| window.render_frame(cx))
            .expect("window");
        let back = cx.read(|cx| editor.read(cx).text(cx));
        assert_eq!(back, sample, "loaded text must come back byte-identical");
    }

    // And through the real open → type → save path.
    let id = open_note_with(&app, cx, "Round trip", samples[3]);
    cx.update_window(app.window, |_, window, cx| {
        app.shell
            .read(cx)
            .notes()
            .clone()
            .update(cx, |n, cx| n.focus_body(window, cx));
    })
    .expect("window");
    press(&app, cx, &secondary("end"));
    press(&app, cx, "ctrl-end");
    type_text(&app, cx, "x");
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    let saved = cx.read(|cx| {
        app.shell
            .read(cx)
            .library()
            .read(cx)
            .note(id)
            .expect("load")
            .expect("note")
            .body()
            .to_owned()
    });
    assert_eq!(saved, format!("{}x", samples[3]));
}

#[gpui_kit::test]
fn selecting_text_shows_a_format_bar_that_keeps_focus(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    open_note_with(&app, cx, "Bar", "make this bold");
    let editor = note_editor(&app, cx);
    cx.update_window(app.window, |_, window, cx| {
        editor.update(cx, |e, cx| e.focus(window, cx));
        window.render_frame(cx);
    })
    .expect("window");
    let bar_open = |cx: &mut TestAppContext| cx.read(|cx| editor.read(cx).is_selection_bar_open());
    assert!(!bar_open(cx), "no bar without a selection");

    press(&app, cx, &secondary("a"));
    assert!(bar_open(cx), "selecting shows the bar");

    click(&app, cx, "fmt-bold");
    let body = cx.read(|cx| editor.read(cx).text(cx));
    assert_eq!(body, "**make this bold**");
    let focused = cx
        .update_window(app.window, |_, window, cx| {
            editor.read(cx).is_focused(window, cx)
        })
        .expect("window");
    assert!(focused, "the bar never takes focus from the text");

    // The change is saved like typing.
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    assert!(all_note_titles(&app, cx).contains(&"Bar".to_owned()));

    press(&app, cx, "escape");
    assert!(!bar_open(cx), "Escape hides the bar");
    press(&app, cx, &secondary("a"));
    assert!(bar_open(cx));
    type_text(&app, cx, "new");
    assert!(!bar_open(cx), "typing over the selection hides it");
}

#[gpui_kit::test]
fn markdown_source_mode_toggles_without_changing_text(cx: &mut TestAppContext) {
    use ottero_desktop::actions::ToggleMarkdownSource;
    let app = launch(cx);
    open_note_with(&app, cx, "Source", "# Heading\n**bold**");
    let notes = cx.read(|cx| app.shell.read(cx).notes().clone());
    let source = |cx: &mut TestAppContext| cx.read(|cx| notes.read(cx).is_source_mode(cx));
    assert!(!source(cx), "live preview is the default");
    dispatch(&app, cx, ToggleMarkdownSource);
    assert!(source(cx));
    assert_eq!(
        cx.read(|cx| notes.read(cx).body_text(cx)),
        "# Heading\n**bold**"
    );
    dispatch(&app, cx, ToggleMarkdownSource);
    assert!(!source(cx));
}

#[gpui_kit::test]
fn clicking_links_and_tags_in_the_editor_navigates(cx: &mut TestAppContext) {
    let app = launch(cx);
    let car = open_note_with(&app, cx, "Car", "Insurance");
    open_note_with(&app, cx, "Trip", "Drive the [[Car]] to #coast");
    let editor = note_editor(&app, cx);
    let engine = cx.read(|cx| editor.read(cx).engine().clone());
    let notes = cx.read(|cx| app.shell.read(cx).notes().clone());

    cx.update(|cx| {
        engine.update(cx, |_, cx| {
            cx.emit(zorite_editor::EditorEvent::OpenWikiLink("coast".into()))
        })
    });
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| notes.read(cx).tag_filter().map(str::to_owned)),
        Some("coast".to_owned()),
        "a #tag filters the list"
    );

    cx.update(|cx| {
        engine.update(cx, |_, cx| {
            cx.emit(zorite_editor::EditorEvent::OpenWikiLink("Car".into()))
        })
    });
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| notes.read(cx).open_note_id()),
        Some(car),
        "a [[link]] opens the note"
    );
}

#[gpui_kit::test]
fn arrow_keys_move_through_math_blocks(cx: &mut TestAppContext) {
    let app = launch(cx);
    open_note_with(&app, cx, "Math", "above\n$$\nx^2\n$$\nbelow");
    let editor = note_editor(&app, cx);
    let engine = cx.read(|cx| editor.read(cx).engine().clone());
    cx.update_window(app.window, |_, window, cx| {
        editor.update(cx, |e, cx| e.focus(window, cx));
        window.render_frame(cx);
    })
    .expect("window");
    let cursor = |cx: &mut TestAppContext| cx.read(|cx| engine.read(cx).cursor());
    assert_eq!(cursor(cx), 0);
    press(&app, cx, "down");
    let inside = cursor(cx);
    assert!(
        (6..=15).contains(&inside),
        "the caret enters the formula instead of stopping: {inside}"
    );
    for _ in 0..4 {
        press(&app, cx, "down");
    }
    assert!(
        cursor(cx) >= "above\n$$\nx^2\n$$\n".len(),
        "and leaves it again"
    );
}

#[gpui_kit::test]
fn find_in_note_counts_steps_wraps_and_closes(cx: &mut TestAppContext) {
    use ottero_desktop::actions::{FindInNote, FindNext, FindPrevious};
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    dispatch(&app, cx, GoNotes);
    open_note_with(
        &app,
        cx,
        "Groceries",
        "Milk, eggs.\nMore milk later.\nNo MILK at all",
    );
    let editor = note_editor(&app, cx);
    let status = |cx: &mut TestAppContext| cx.read(|cx| editor.read(cx).find_status());

    assert_eq!(status(cx), None, "closed until asked for");
    dispatch(&app, cx, FindInNote);
    assert_eq!(status(cx), Some((0, 0)), "open with an empty query");
    type_text(&app, cx, "milk");
    assert_eq!(
        status(cx),
        Some((1, 3)),
        "case-insensitive, active on the first"
    );

    dispatch(&app, cx, FindNext);
    assert_eq!(status(cx), Some((2, 3)));
    press(&app, cx, "enter");
    assert_eq!(
        status(cx),
        Some((3, 3)),
        "Enter in the field goes to the next"
    );
    dispatch(&app, cx, FindNext);
    assert_eq!(status(cx), Some((1, 3)), "wraps to the first");
    dispatch(&app, cx, FindPrevious);
    assert_eq!(status(cx), Some((3, 3)), "and back to the last");
    press(&app, cx, "shift-enter");
    assert_eq!(status(cx), Some((2, 3)));

    // Edits keep the count current.
    cx.update(|cx| {
        let engine = editor.read(cx).engine().clone();
        engine.update(cx, |e, cx| e.replace_range(0..4, "Oat", cx));
        engine.update(cx, |_, cx| cx.emit(zorite_editor::EditorEvent::Changed));
    });
    cx.run_until_parked();
    assert_eq!(status(cx).map(|s| s.1), Some(2), "matches follow the text");

    press(&app, cx, "escape");
    assert_eq!(status(cx), None, "Escape closes the bar");
    let focused = cx
        .update_window(app.window, |_, window, cx| {
            editor.read(cx).is_focused(window, cx)
        })
        .expect("window");
    assert!(focused, "and puts you back in the text");

    // ⌘F with a selection searches for it.
    press(&app, cx, &secondary("a"));
    cx.update(|cx| {
        let engine = editor.read(cx).engine().clone();
        engine.update(cx, |e, cx| e.set_cursor(0, cx));
    });
    cx.run_until_parked();
    dispatch(&app, cx, FindInNote);
    type_text(&app, cx, "zzz");
    assert_eq!(status(cx), Some((0, 0)), "no match shows zero");
}

/// The OS checker exists on macOS and Windows; elsewhere spelling is a no-op.
#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn spelling_flags_prose_skips_code_and_can_be_turned_off(cx: &mut TestAppContext) {
    let app = launch(cx);
    dispatch(&app, cx, GoNotes);
    let body = "I recieved the packige.\n`teh code` and [[Grocerys]] #teh";
    open_note_with(&app, cx, "Spelling", body);
    let editor = note_editor(&app, cx);
    let flagged = |cx: &mut TestAppContext| cx.read(|cx| editor.read(cx).flagged_words(cx));
    let words = flagged(cx);
    assert!(words.contains(&"recieved".to_owned()), "{words:?}");
    assert!(words.contains(&"packige".to_owned()), "{words:?}");
    assert!(
        !words
            .iter()
            .any(|w| ["teh", "Grocerys"].contains(&w.as_str())),
        "code, links and tags are not prose: {words:?}"
    );

    cx.update(|cx| ottero_ui::appearance::update(cx, |s| s.spell_check = false));
    cx.run_until_parked();
    assert!(flagged(cx).is_empty(), "the setting turns underlines off");
}

#[gpui_kit::test]
fn picking_a_theme_card_applies_and_saves_it(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    dispatch(&app, cx, UseDarkTheme);
    dispatch(&app, cx, GoSettings);
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        w.within("settings-sidebar").click("0-3", cx);
    })
    .expect("Appearance navigation");
    cx.run_until_parked();
    let name = |cx: &mut TestAppContext| cx.read(|cx| Theme::global(cx).theme_name().to_string());
    assert_eq!(name(cx), ottero_ui::theme::DARK);

    click(&app, cx, "theme-catppuccin-mocha");
    assert_eq!(name(cx), "Catppuccin Mocha");
    assert_eq!(
        cx.read(|cx| Theme::global(cx).highlight_theme.name.clone()),
        "Catppuccin Mocha",
        "code colors follow the theme"
    );

    // A light card switches to light mode so the choice is visible.
    click(&app, cx, "theme-solarized-light");
    assert_eq!(cx.read(|cx| Theme::global(cx).mode), ThemeMode::Light);
    assert_eq!(name(cx), "Solarized Light");

    let saved = ottero_ui::appearance::AppearanceSettings::load(&app.paths.appearance);
    assert_eq!(saved.dark_theme, "Catppuccin Mocha");
    assert_eq!(saved.light_theme, "Solarized Light");

    // Back to dark: the chosen dark theme comes back, not the default.
    dispatch(&app, cx, UseDarkTheme);
    assert_eq!(name(cx), "Catppuccin Mocha");
}

#[gpui_kit::test]
fn calendar_quick_add_keyboard_views_and_moves(cx: &mut TestAppContext) {
    use ottero_desktop::actions::{GoCalendar, NewEvent};
    use ottero_desktop::views::calendar_layout::CalendarMode;

    let app = launch(cx);
    dispatch(&app, cx, GoCalendar);
    assert_eq!(route(&app, cx), Route::Calendar);
    let calendar = cx.read(|cx| app.shell.read(cx).calendar().clone());
    let today = ottero_core::clock::today();

    // Quick add from the "New event" command.
    dispatch(&app, cx, NewEvent);
    type_text(&app, cx, "Lunch with Sam tomorrow 1pm");
    // Calendar keys must not fire while typing ("w", "t", "m" are in the text).
    assert_eq!(cx.read(|cx| calendar.read(cx).mode()), CalendarMode::Month);
    press(&app, cx, "enter");
    let tomorrow = today.tomorrow().expect("date");
    let span = cx.read(|cx| calendar.read(cx).span().clone());
    let occ = span
        .on_day(tomorrow)
        .next()
        .copied()
        .expect("event shown on its day");
    let event = span.events.get(&occ.event).expect("event");
    assert_eq!(event.title(), "Lunch with Sam");
    assert_eq!(
        event.start(),
        tomorrow.to_datetime(ottero_core::Time::constant(13, 0, 0, 0))
    );
    assert_eq!(
        cx.read(|cx| calendar.read(cx).anchor()),
        tomorrow,
        "selects where it landed"
    );

    // Grid keys once the grid has focus.
    dispatch(&app, cx, GoCalendar);
    press(&app, cx, "w");
    assert_eq!(cx.read(|cx| calendar.read(cx).mode()), CalendarMode::Week);
    press(&app, cx, "t");
    assert_eq!(cx.read(|cx| calendar.read(cx).anchor()), today);
    press(&app, cx, "right");
    assert_eq!(cx.read(|cx| calendar.read(cx).anchor()), tomorrow);
    press(&app, cx, "d");
    assert_eq!(cx.read(|cx| calendar.read(cx).mode()), CalendarMode::Day);
    press(&app, cx, "a");
    assert_eq!(cx.read(|cx| calendar.read(cx).mode()), CalendarMode::Agenda);
    press(&app, cx, "m");
    assert_eq!(cx.read(|cx| calendar.read(cx).mode()), CalendarMode::Month);

    // Moving it (what a drag does) refreshes the view.
    let id = event.id();
    let later = tomorrow.tomorrow().expect("date");
    cx.update(|cx| {
        app.shell.read(cx).library().clone().update(cx, |lib, cx| {
            lib.move_event(id, later, cx).expect("move");
        })
    });
    cx.run_until_parked();
    let span = cx.read(|cx| calendar.read(cx).span().clone());
    assert!(span.on_day(tomorrow).next().is_none());
    assert_eq!(span.on_day(later).next().map(|o| o.event), Some(id));
}

#[gpui_kit::test]
fn every_calendar_view_renders_events_and_tasks(cx: &mut TestAppContext) {
    use ottero_desktop::actions::GoCalendar;
    use ottero_desktop::views::calendar_layout::CalendarMode;

    let app = launch(cx);
    let today = ottero_core::clock::today();
    cx.update(|cx| {
        app.shell.read(cx).library().clone().update(cx, |lib, cx| {
            for text in [
                "Standup 9:30-9:45 every weekday",
                "Review 9-10am",
                "Overlap 9:30-11am",
                "Trip nov 20-22",
                "Flight 10pm-2am",
            ] {
                lib.quick_add_event(text, today, cx).expect("event");
            }
            lib.create_task("Pay rent", Some(today), cx).expect("task");
        })
    });
    dispatch(&app, cx, GoCalendar);
    let calendar = cx.read(|cx| app.shell.read(cx).calendar().clone());
    for (mode, _) in CalendarMode::OPTIONS {
        cx.update(|cx| calendar.update(cx, |c, cx| c.set_mode(mode, cx)));
        cx.run_until_parked();
        cx.update_window(app.window, |_, window, _| window.refresh())
            .expect("window");
        cx.run_until_parked();
        let span = cx.read(|cx| calendar.read(cx).span().clone());
        assert!(
            span.on_day(today).count() >= 3,
            "{mode:?} shows today's events"
        );
        assert_eq!(span.tasks_on(today).count(), 1, "{mode:?} shows the task");
    }
}

#[gpui_kit::test]
fn habits_quick_add_check_in_keyboard_and_archive(cx: &mut TestAppContext) {
    use ottero_core::habit::Schedule;
    use ottero_desktop::actions::{GoHabits, NewHabit};

    let app = launch(cx);
    dispatch(&app, cx, NewHabit);
    assert_eq!(route(&app, cx), Route::Habits);
    type_text(&app, cx, "Drink water 3 times a day");
    press(&app, cx, "enter");
    type_text(&app, cx, "Run 3x a week");
    press(&app, cx, "enter");
    let habits = cx.read(|cx| app.shell.read(cx).habits().clone());
    let list = cx.read(|cx| habits.read(cx).habits().to_vec());
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].habit.name(), "Drink water");
    assert_eq!(list[0].habit.target(), 3);
    assert_eq!(list[1].habit.schedule(), Schedule::PerWeek(3));

    // Keyboard: select the first habit, Space counts up toward the goal.
    dispatch(&app, cx, GoHabits);
    press(&app, cx, "down");
    press(&app, cx, "space");
    press(&app, cx, "space");
    let water = cx.read(|cx| habits.read(cx).habits()[0].clone());
    assert_eq!(water.today_count, 2);
    assert!(!water.done_today());
    press(&app, cx, "space");
    let water = cx.read(|cx| habits.read(cx).habits()[0].clone());
    assert!(water.done_today());
    assert_eq!(water.stats.current_streak, 1);

    // Archiving moves it out of the active list; the history stays.
    let id = water.habit.id();
    cx.update(|cx| {
        app.shell.read(cx).library().clone().update(cx, |lib, cx| {
            lib.archive_habit(id, cx).expect("archive");
        })
    });
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| habits.read(cx).habits().len()), 1);
    cx.update(|cx| habits.update(cx, |h, cx| h.set_archived(true, cx)));
    let archived = cx.read(|cx| habits.read(cx).habits().to_vec());
    assert_eq!(archived[0].today_count, 3, "history kept");

    // Future check-ins are refused.
    let tomorrow = ottero_core::clock::today().tomorrow().expect("date");
    let other = list[1].habit.id();
    let err = cx.update(|cx| {
        app.shell
            .read(cx)
            .library()
            .clone()
            .update(cx, |lib, cx| lib.tap_habit(other, tomorrow, cx).is_err())
    });
    assert!(err);
}

#[gpui_kit::test]
fn habit_insights_render_with_history(cx: &mut TestAppContext) {
    use ottero_desktop::actions::GoHabits;
    use ottero_desktop::views::habits::HabitTab;

    let app = launch(cx);
    let today = ottero_core::clock::today();
    cx.update(|cx| {
        app.shell.read(cx).library().clone().update(cx, |lib, cx| {
            let read = lib.quick_add_habit("Read daily", cx).expect("habit");
            lib.quick_add_habit("Run 3x a week", cx).expect("habit");
            for k in 1..20 {
                let day = today.checked_add(jiff::Span::new().days(-k)).expect("date");
                lib.set_habit_count(read, day, 1, cx).expect("check-in");
            }
        })
    });
    dispatch(&app, cx, GoHabits);
    let habits = cx.read(|cx| app.shell.read(cx).habits().clone());
    cx.update(|cx| habits.update(cx, |h, cx| h.set_tab(HabitTab::Insights, cx)));
    cx.update_window(app.window, |_, window, _| window.refresh())
        .expect("window");
    cx.run_until_parked();
    let read = cx.read(|cx| habits.read(cx).habits()[0].clone());
    assert_eq!(read.stats.current_streak, 19, "backfilled days count");
    assert!(read.stats.strength > 0.5);
    // Keyboard selection does nothing on Insights (no rows to act on).
    press(&app, cx, "down");
    assert_eq!(cx.read(|cx| habits.read(cx).selected_habit()), None);
}

#[gpui_kit::test]
fn calendar_routines_can_be_hidden(cx: &mut TestAppContext) {
    use ottero_desktop::actions::GoCalendar;

    let app = launch(cx);
    let today = ottero_core::clock::today();
    cx.update(|cx| {
        app.shell.read(cx).library().clone().update(cx, |lib, cx| {
            lib.quick_add_event("Gym 7am every day", today, cx)
                .expect("event");
            lib.quick_add_event("Dentist 4pm", today, cx)
                .expect("event");
        })
    });
    dispatch(&app, cx, GoCalendar);
    let calendar = cx.read(|cx| app.shell.read(cx).calendar().clone());
    let shown = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            let c = calendar.read(cx);
            c.span()
                .on_day(today)
                .filter(|o| c.shows_for_test(o, cx))
                .count()
        })
    };
    assert_eq!(shown(cx), 2);
    cx.update(|cx| ottero_ui::appearance::update(cx, |s| s.calendar_routines = false));
    cx.update_window(app.window, |_, window, _| window.refresh())
        .expect("window");
    cx.run_until_parked();
    assert_eq!(shown(cx), 1, "only the one-off dentist visit");
}

#[gpui_kit::test]
fn today_dashboard_shows_schedule_habits_and_respects_layout(cx: &mut TestAppContext) {
    use ottero_desktop::views::today::set_shown;
    use ottero_desktop::views::today_layout::TodaySection;

    let app = launch(cx);
    let lib = library(&app, cx);
    let today = ottero_core::clock::today();
    let (event, habit) = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let e = l
                .quick_add_event("Dentist 11:30pm", today, cx)
                .expect("event");
            let h = l.quick_add_habit("Read daily", cx).expect("habit");
            (e, h)
        })
    });
    dispatch(&app, cx, GoToday);
    let find = |cx: &mut TestAppContext, id: String| {
        cx.update_window(app.window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(leak(id)).is_some()
        })
        .expect("window")
    };
    assert!(
        find(cx, format!("today-event-{event}")),
        "event in today's schedule"
    );
    assert!(find(cx, "today-glance".into()));
    assert!(
        find(cx, format!("habit-check-{habit}")),
        "habit check on Today"
    );

    // Checking the habit from Today saves it.
    cx.update_window(app.window, |_, window, cx| {
        window.render_frame(cx);
        let id: &'static str = leak(format!("habit-check-{habit}"));
        window.click(id, cx);
    })
    .expect("window");
    cx.run_until_parked();
    let done = cx.read(|cx| lib.read(cx).habits(false).expect("habits")[0].done_today());
    assert!(done, "checked from Today");

    // Hiding a section removes it; the setting persists in appearance.
    cx.update(|cx| set_shown(TodaySection::Agenda, false, cx));
    cx.run_until_parked();
    assert!(!find(cx, "today-agenda".into()));
    assert!(find(cx, "today-habits".into()));
}

// ---- Password vault ----

#[gpui_kit::test]
fn vault_setup_items_lock_unlock_and_copy(cx: &mut TestAppContext) {
    use ottero_core::vault::lock::LockState;
    use ottero_core::vault::{ItemFields, ItemKind};
    use ottero_desktop::actions::{GoPasswords, LockVault};

    let app = launch(cx);
    let lib = library(&app, cx);
    cx.update(|cx| {
        lib.update(cx, |l, _| {
            l.set_kdf_params_for_tests(ottero_crypto::KdfParams::MINIMUM)
        })
    });
    dispatch(&app, cx, GoPasswords);
    assert_eq!(route(&app, cx), Route::Passwords);
    assert_eq!(
        cx.read(|cx| lib.read(cx).vault_state()),
        LockState::Uninitialized
    );

    // Setup: the master field has focus; a weak password can't be used.
    let passwords = cx.read(|cx| app.shell.read(cx).passwords().clone());
    type_text(&app, cx, "short");
    click(&app, cx, "vault-create");
    assert_eq!(
        cx.read(|cx| lib.read(cx).vault_state()),
        LockState::Uninitialized
    );

    let master = cx.read(|cx| passwords.read(cx).master_input().clone());
    cx.update_window(app.window, |_, window, cx| {
        master.update(cx, |i, cx| {
            i.set_value("plum-orbit-candle-9-meadow", window, cx)
        });
    })
    .expect("window");
    cx.run_until_parked();
    // Tab to confirm, type it again, tick the box, create.
    press(&app, cx, "tab");
    type_text(&app, cx, "plum-orbit-candle-9-meadow");
    click(&app, cx, "vault-ack");
    click(&app, cx, "vault-create");
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| lib.read(cx).vault_state()),
        LockState::Unlocked
    );
    // The recovery key is shown once; Continue needs the "saved it" box.
    let key = cx.read(|cx| passwords.read(cx).recovery_key().map(str::to_owned));
    assert!(key.is_some_and(|k| k.len() == 39), "XXXX-… ×8");
    assert!(cx.read(|cx| lib.read(cx).vault_has_recovery()));
    click(&app, cx, "recovery-done");
    assert!(
        cx.read(|cx| passwords.read(cx).recovery_key().is_some()),
        "not yet acknowledged"
    );
    click(&app, cx, "recovery-saved");
    click(&app, cx, "recovery-done");
    assert!(cx.read(|cx| passwords.read(cx).recovery_key().is_none()));

    // Items.
    let id = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let fields = ItemFields {
                title: "GitHub".into(),
                username: "sam".into(),
                password: "Tr0ub4dor&3-horse".into(),
                urls: vec!["https://github.com".into()],
                ..ItemFields::new(ItemKind::Login)
            };
            l.create_vault_item(&fields, cx).expect("create")
        })
    });
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| passwords.read(cx).ids().to_vec()), vec![id]);

    // Select it from the search field with ↓, then ⌘C copies the password.
    dispatch(&app, cx, GoPasswords);
    press(&app, cx, "down");
    assert_eq!(cx.read(|cx| passwords.read(cx).selected()), Some(id));
    press(&app, cx, leak(secondary("c")));
    let copied = cx.read(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(copied.as_deref(), Some("Tr0ub4dor&3-horse"));
    // …and it clears after the delay.
    cx.executor().advance_clock(Duration::from_secs(31));
    cx.run_until_parked();
    let copied = cx.read(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert!(copied.unwrap_or_default().is_empty(), "cleared");

    // Lock drops everything; a wrong password doesn't unlock.
    dispatch(&app, cx, LockVault);
    assert_eq!(cx.read(|cx| lib.read(cx).vault_state()), LockState::Locked);
    assert!(cx.read(|cx| lib.read(cx).vault().items().is_empty()));
    dispatch(&app, cx, GoPasswords);
    type_text(&app, cx, "not the password");
    press(&app, cx, "enter");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| lib.read(cx).vault_state()), LockState::Locked);

    // The right one does, and the item is back.
    cx.update_window(app.window, |_, window, cx| {
        window.dispatch_action(Box::new(ottero_desktop::actions::GoPasswords), cx)
    })
    .expect("window");
    cx.run_until_parked();
    let unlock_task = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.unlock_vault(
                ottero_crypto::SecretString::from("plum-orbit-candle-9-meadow".to_owned()),
                cx,
            )
        })
    });
    unlock_task.detach();
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| lib.read(cx).vault_state()),
        LockState::Unlocked
    );
    assert_eq!(
        cx.read(|cx| lib.read(cx).vault_item(id).map(|i| i.title().to_owned()))
            .as_deref(),
        Some("GitHub")
    );
}

#[gpui_kit::test]
fn vault_auto_locks_after_idle(cx: &mut TestAppContext) {
    use ottero_core::vault::lock::{AutoLock, LockState};
    use ottero_desktop::actions::GoPasswords;

    let app = launch(cx);
    let lib = library(&app, cx);
    cx.update(|cx| {
        lib.update(cx, |l, _| {
            l.set_kdf_params_for_tests(ottero_crypto::KdfParams::MINIMUM)
        });
        ottero_desktop::vault_prefs::update(cx, |p| p.auto_lock = AutoLock::OneMinute);
    });
    dispatch(&app, cx, GoPasswords);
    let setup = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.set_up_vault(
                ottero_crypto::SecretString::from("plum-orbit-candle-9-meadow".to_owned()),
                cx,
            )
        })
    });
    setup.detach();
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| lib.read(cx).vault_state()),
        LockState::Unlocked
    );
    // Idle is measured on the real clock; pretend the last activity was long ago.
    cx.update(|cx| {
        lib.update(cx, |l, _| {
            l.backdate_vault_activity_for_tests(Duration::from_secs(120))
        })
    });
    cx.executor().advance_clock(Duration::from_secs(11));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| lib.read(cx).vault_state()), LockState::Locked);
}

/// An unlocked vault on the Passwords screen (fast KDF).
fn unlocked_vault(cx: &mut TestAppContext) -> App {
    use ottero_desktop::actions::GoPasswords;
    let app = launch(cx);
    let lib = library(&app, cx);
    cx.update(|cx| {
        lib.update(cx, |l, _| {
            l.set_kdf_params_for_tests(ottero_crypto::KdfParams::MINIMUM)
        })
    });
    dispatch(&app, cx, GoPasswords);
    let setup = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.set_up_vault(
                ottero_crypto::SecretString::from("plum-orbit-candle-9-meadow".to_owned()),
                cx,
            )
        })
    });
    setup.detach();
    cx.run_until_parked();
    app
}

fn element_present(app: &App, cx: &mut TestAppContext, id: &'static str) -> bool {
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).is_some()
    })
    .expect("window")
}

#[gpui_kit::test]
fn sidebar_customization_reorders_hides_persists_and_resets_with_undo(cx: &mut TestAppContext) {
    use ottero_desktop::actions::CustomizeSidebar;
    use ottero_desktop::ui_state::{LayoutState, layout_file};
    let app = launch(cx);
    let prefs = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            app.shell
                .read(cx)
                .layout(cx)
                .navigation
                .expect("preferences")
        })
    };
    assert_eq!(prefs(cx).visible, ["today", "notes", "tasks", "calendar"]);
    dispatch(&app, cx, CustomizeSidebar);
    assert!(element_present(&app, cx, "sidebar-show-focus"));
    click(&app, cx, "sidebar-show-focus");
    assert!(prefs(cx).is_visible("focus"));
    // Keyboard activation uses the same native move button as a pointer click.
    let focused = |cx: &mut TestAppContext| {
        cx.update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            w.find("sidebar-up-notes").focused() == Some(true)
        })
        .expect("focus query")
    };
    for _ in 0..40 {
        if focused(cx) {
            break;
        }
        press(&app, cx, "tab");
    }
    assert!(focused(cx), "move controls are keyboard reachable");
    press(&app, cx, "space");
    assert_eq!(prefs(cx).ordered()[0].id, "notes");
    assert!(focused(cx), "keyboard focus stays with the moved control");
    click(&app, cx, "sidebar-down-notes");
    assert_eq!(prefs(cx).ordered()[0].id, "today");
    click(&app, cx, "sidebar-up-notes");
    let customized = prefs(cx);
    cx.update(|cx| {
        ottero_ui::appearance::reset(cx);
        cx.set_reduce_motion(true);
    });
    cx.run_until_parked();
    assert_eq!(
        prefs(cx),
        customized,
        "theme reset preserves workspace choices"
    );
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    assert_eq!(
        LayoutState::load(&layout_file(&app.paths.data_dir)).navigation,
        Some(customized.clone())
    );
    click(&app, cx, "sidebar-reset");
    assert_eq!(prefs(cx).visible, ["today", "notes", "tasks", "calendar"]);
    click(&app, cx, "undo");
    assert_eq!(prefs(cx), customized);
}

#[gpui_kit::test]
fn sidebar_more_is_inline_and_all_hidden_still_has_navigation(cx: &mut TestAppContext) {
    use ottero_desktop::actions::{CustomizeSidebar, GoPasswords};
    let app = launch(cx);
    // More sits immediately after four default features, not in the footer.
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        let more = w.within("main-sidebar-220").find("0-0-4").bounds();
        let settings = w.find("sidebar-settings").bounds();
        assert!(more.top() < settings.top() / 2.);
        w.within("main-sidebar-220").click("0-0-4", cx);
    })
    .expect("More");
    cx.run_until_parked();
    dispatch(&app, cx, CustomizeSidebar);
    for entry in ottero_desktop::nav::entries() {
        let visible = cx.read(|cx| {
            app.shell
                .read(cx)
                .layout(cx)
                .navigation
                .expect("prefs")
                .is_visible(entry.id)
        });
        if visible {
            click(&app, cx, leak(format!("sidebar-show-{}", entry.id)));
        }
    }
    assert!(element_present(&app, cx, "open-palette"));
    assert!(element_present(&app, cx, "sidebar-settings"));
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        w.within("main-sidebar-220").click("0-0-0", cx);
    })
    .expect("all hidden More");
    cx.run_until_parked();
    // Keyboard commands remain valid even with no features pinned.
    dispatch(&app, cx, GoPasswords);
    assert_eq!(route(&app, cx), Route::Passwords);
    dispatch(&app, cx, CustomizeSidebar);
    for entry in ottero_desktop::nav::entries() {
        click(&app, cx, leak(format!("sidebar-show-{}", entry.id)));
    }
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert!(
            w.within("main-sidebar-220").try_find("0-0-8").is_none(),
            "no empty More when all features are visible"
        );
    })
    .expect("all visible");
}

#[gpui_kit::test]
fn hiding_focus_keeps_timer_running_and_feature_settings_are_searchable(cx: &mut TestAppContext) {
    use ottero_desktop::actions::CustomizeSidebar;
    let app = launch(cx);
    dispatch(&app, cx, GoFocus);
    dispatch(&app, cx, FocusToggle);
    let ctrl = cx.read(|cx| app.shell.read(cx).focus_ctrl().clone());
    let running = cx.read(|cx| ctrl.read(cx).timer().clone());
    dispatch(&app, cx, CustomizeSidebar);
    click(&app, cx, "sidebar-show-focus");
    click(&app, cx, "sidebar-show-focus");
    assert!(cx.read(|cx| ctrl.read(cx).timer().is_running()));
    assert_eq!(cx.read(|cx| ctrl.read(cx).timer().kind), running.kind);
    click_placeholder(&app, cx, "Search...");
    type_text(&app, cx, "spellcheck");
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        use gpui_kit::test::TestQueryExt as _;
        assert!(
            !w.elements()
                .into_iter()
                .filter(|e| e.role() == Some(gpui_kit::Role::Switch))
                .collect::<Vec<_>>()
                .is_empty(),
            "global settings search reaches Notes preferences"
        );
    })
    .expect("feature search");
}

#[gpui_kit::test]
fn workspace_start_page_loads_and_can_be_changed_without_pinning_it(cx: &mut TestAppContext) {
    use ottero_desktop::{
        navigation::SidebarPreferences,
        ui_state::{LayoutState, layout_file},
    };
    let dir = tempfile::tempdir().expect("dir");
    let paths = AppPaths::in_dir(dir.path().to_path_buf());
    let prefs = SidebarPreferences {
        start_page: "notes".into(),
        visible: vec!["notes".into()],
        ..SidebarPreferences::default()
    };
    LayoutState {
        navigation: Some(prefs),
        ..LayoutState::default()
    }
    .save(&layout_file(dir.path()));
    let (window, shell) = cx.update(|cx| {
        ottero_desktop::start_with_account_api(
            &paths,
            Store::open_in_memory().expect("store"),
            offline_account_api(),
            cx,
        )
        .expect("start")
    });
    cx.run_until_parked();
    let app = App {
        window,
        shell,
        _dir: dir,
        paths,
    };
    assert_eq!(route(&app, cx), Route::Notes);
    dispatch(&app, cx, GoSettings);
    // Settings opens on Account; General is the second page.
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        w.within("settings-sidebar").click("0-1", cx);
    })
    .expect("General navigation");
    cx.run_until_parked();
    click_label(&app, cx, "Open on startup: Notes");
    click_label(&app, cx, "Calendar");
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    let saved = LayoutState::load(&layout_file(&app.paths.data_dir))
        .navigation
        .expect("prefs");
    assert_eq!(saved.startup_route(), Route::Calendar);
    assert!(
        !saved.is_visible("calendar"),
        "startup does not silently pin a hidden feature"
    );
    assert_eq!(
        route(&app, cx),
        Route::Settings,
        "startup choice applies on next launch"
    );
}

#[gpui_kit::test]
fn sidebar_hiding_preserves_note_drafts_and_vault_state(cx: &mut TestAppContext) {
    use ottero_core::vault::lock::LockState;
    use ottero_desktop::actions::CustomizeSidebar;
    let app = unlocked_vault(cx);
    let lib = library(&app, cx);
    dispatch(&app, cx, NewNote);
    type_text(&app, cx, "Private workspace");
    press(&app, cx, "enter");
    type_text(&app, cx, "Keep this draft intact.");
    let notes = cx.read(|cx| app.shell.read(cx).notes().clone());
    let body = cx.read(|cx| notes.read(cx).body_text(cx));
    dispatch(&app, cx, CustomizeSidebar);
    click(&app, cx, "sidebar-show-notes");
    click(&app, cx, "sidebar-show-passwords");
    click(&app, cx, "sidebar-show-passwords");
    assert_eq!(
        cx.read(|cx| lib.read(cx).vault_state()),
        LockState::Unlocked
    );
    assert_eq!(cx.read(|cx| notes.read(cx).body_text(cx)), body);
    dispatch(&app, cx, GoNotes);
    assert_eq!(cx.read(|cx| notes.read(cx).body_text(cx)), body);
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    let note = cx.read(|cx| lib.read(cx).notes(false).expect("notes").remove(0));
    assert_eq!(
        cx.read(|cx| lib
            .read(cx)
            .note(note.id)
            .expect("read")
            .expect("note")
            .body()
            .to_owned()),
        body
    );
}

fn click_placeholder(app: &App, cx: &mut TestAppContext, placeholder: &str) {
    use gpui_kit::test::{TestEventExt as _, TestQueryExt as _};
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        let inputs: Vec<_> = w
            .elements()
            .into_iter()
            .filter(|e| e.placeholder() == Some(placeholder))
            .collect();
        assert_eq!(inputs.len(), 1, "one matching input");
        let center = inputs[0].bounds().center();
        w.pointer_down(center, gpui_kit::MouseButton::Left, cx);
        w.pointer_up(center, gpui_kit::MouseButton::Left, cx);
    })
    .expect("window");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn developer_forms_leave_cleanly_and_unlock_guidance_is_contextual(cx: &mut TestAppContext) {
    use gpui_kit::test::TestQueryExt as _;
    use ottero_desktop::actions::{GoDeveloper, LockVault, NewDeveloperProject, NewProjectSecrets};
    let app = unlocked_vault(cx);
    let lib = library(&app, cx);
    let project = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.create_developer_project("Website", None, cx)
                .expect("project")
        })
    });
    dispatch(&app, cx, NewDeveloperProject);
    assert!(element_present(&app, cx, "developer-save"));
    press(&app, cx, "escape");
    assert!(!element_present(&app, cx, "developer-save"));
    dispatch(&app, cx, NewDeveloperProject);
    click(&app, cx, leak(format!("developer-project-{project}")));
    assert!(
        !element_present(&app, cx, "developer-save"),
        "selecting a project leaves create form"
    );
    dispatch(&app, cx, NewDeveloperProject);
    click(&app, cx, "developer-more");
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(
            w.find_by_label("Show archived projects").checked(),
            Some(false)
        );
    })
    .expect("menu");
    click_label(&app, cx, "Show archived projects");
    assert!(!element_present(&app, cx, "developer-save"));
    click(&app, cx, "developer-more");
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(
            w.find_by_label("Show archived projects").checked(),
            Some(true)
        );
    })
    .expect("menu");
    click_label(&app, cx, "Show archived projects");
    dispatch(&app, cx, NewDeveloperProject);
    dispatch(&app, cx, NewProjectSecrets);
    assert!(!element_present(&app, cx, "developer-save"));
    assert!(
        element_present(&app, cx, "edit-save"),
        "new environment is visible"
    );
    let developer = cx.read(|cx| app.shell.read(cx).developer().clone());
    let pane = cx.read(|cx| developer.read(cx).pane().clone());
    cx.update_window(app.window, |_, w, cx| {
        pane.update(cx, |p, cx| p.cancel(w, cx))
    })
    .expect("cancel");
    dispatch(&app, cx, LockVault);
    dispatch(&app, cx, NewProjectSecrets);
    assert!(element_present(&app, cx, "developer-unlock"));
    let task = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.unlock_vault(
                ottero_crypto::SecretString::from("plum-orbit-candle-9-meadow".to_owned()),
                cx,
            )
        })
    });
    task.detach();
    cx.run_until_parked();
    dispatch(&app, cx, GoDeveloper);
    assert!(
        !element_present(&app, cx, "developer-unlock"),
        "unlock guidance disappears"
    );
    dispatch(&app, cx, NewProjectSecrets);
    assert!(element_present(&app, cx, "edit-save"));
}

#[gpui_kit::test]
fn passwords_navigation_preserves_invalid_environment_drafts(cx: &mut TestAppContext) {
    use ottero_core::vault::environment::EnvironmentSecrets;
    use ottero_core::vault::{ItemFields, ItemKind};
    use ottero_desktop::actions::{GoPasswords, NewLogin};
    use ottero_desktop::vault::VaultFilter;
    let app = unlocked_vault(cx);
    let lib = library(&app, cx);
    let id = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.create_vault_item(
                &ItemFields {
                    title: "Other".into(),
                    ..ItemFields::new(ItemKind::Login)
                },
                cx,
            )
            .expect("other");
            l.create_vault_item(
                &ItemFields {
                    title: "Development".into(),
                    environment: Some(EnvironmentSecrets::parse("OLD=value").expect("parse")),
                    ..ItemFields::new(ItemKind::Note)
                },
                cx,
            )
            .expect("environment")
        })
    });
    dispatch(&app, cx, GoPasswords);
    let passwords = cx.read(|cx| app.shell.read(cx).passwords().clone());
    let pane = cx.read(|cx| passwords.read(cx).pane().clone());
    let editor = cx.read(|cx| pane.read(cx).environment_editor().clone());
    for pending in [true, false] {
        cx.update_window(app.window, |_, w, cx| {
            passwords.update(cx, |p, cx| p.select_for_dev("Development", w, cx));
            pane.update(cx, |p, cx| p.edit(w, cx));
            if pending {
                editor.update(cx, |e, cx| e.preview("NEW=value", cx));
            }
        })
        .expect("edit");
        cx.run_until_parked();
        if !pending {
            click_placeholder(&app, cx, "VARIABLE_NAME");
            press(&app, cx, leak(secondary("a")));
            type_text(&app, cx, "BAD-KEY");
        }
        assert!(cx.read(|cx| editor.read(cx).document(cx).is_err()));
        let can_restart = cx
            .update_window(app.window, |_, w, cx| {
                app.shell
                    .update(cx, |shell, cx| shell.prepare_update_restart(w, cx))
            })
            .expect("window");
        assert!(
            !can_restart,
            "pending imports and invalid secrets block update restart"
        );
        assert!(cx.read(|cx| pane.read(cx).is_editing()));
        cx.update_window(app.window, |_, w, cx| {
            passwords.update(cx, |p, cx| p.select_for_dev("Other", w, cx))
        })
        .expect("switch");
        cx.run_until_parked();
        assert_eq!(cx.read(|cx| passwords.read(cx).selected()), Some(id));
        assert!(cx.read(|cx| pane.read(cx).is_editing()));
        dispatch(&app, cx, NewLogin);
        cx.update_window(app.window, |_, w, cx| {
            passwords.update(cx, |p, cx| {
                p.set_filter(VaultFilter::Kind(ItemKind::Card), w, cx)
            })
        })
        .expect("filter");
        assert_eq!(cx.read(|cx| passwords.read(cx).filter()), VaultFilter::All);
        assert!(
            cx.read(|cx| editor.read(cx).document(cx).is_err()),
            "invalid draft still present"
        );
        if pending {
            cx.update_window(app.window, |_, w, cx| {
                pane.update(cx, |p, cx| p.cancel(w, cx))
            })
            .expect("cancel");
        } else {
            click_placeholder(&app, cx, "VARIABLE_NAME");
            press(&app, cx, leak(secondary("a")));
            type_text(&app, cx, "  NEW_KEY  ");
            cx.update_window(app.window, |_, w, cx| {
                passwords.update(cx, |p, cx| p.select_for_dev("Other", w, cx))
            })
            .expect("correct and switch");
            cx.run_until_parked();
            assert_ne!(cx.read(|cx| passwords.read(cx).selected()), Some(id));
            let saved = cx.read(|cx| {
                lib.read(cx)
                    .vault_item(id)
                    .expect("item")
                    .fields()
                    .environment
                    .clone()
                    .expect("environment")
            });
            assert_eq!(saved.variables()[0].key(), "NEW_KEY");
            assert_eq!(saved.variables()[0].value(), "value");
        }
    }
}

#[gpui_kit::test]
fn environment_reveal_is_literal_and_copy_updates_recent_use(cx: &mut TestAppContext) {
    use gpui_kit::test::TestQueryExt as _;
    use ottero_core::vault::environment::EnvironmentSecrets;
    use ottero_core::vault::{ItemFields, ItemKind};
    use ottero_desktop::actions::{CopyProjectEnvironment, GoPasswords};
    let app = unlocked_vault(cx);
    let lib = library(&app, cx);
    let value = "line one\nline two\tC:\\folder";
    let env = EnvironmentSecrets::new(
        None,
        "Development",
        vec![
            ottero_core::vault::environment::EnvVariable::new("PRIVATE_KEY", value).expect("value"),
        ],
    )
    .expect("env");
    let id = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.create_vault_item(
                &ItemFields {
                    title: "Environment".into(),
                    environment: Some(env),
                    ..ItemFields::new(ItemKind::Note)
                },
                cx,
            )
            .expect("item")
        })
    });
    dispatch(&app, cx, GoPasswords);
    let passwords = cx.read(|cx| app.shell.read(cx).passwords().clone());
    cx.update_window(app.window, |_, w, cx| {
        passwords.update(cx, |p, cx| p.select_for_dev("Environment", w, cx))
    })
    .expect("show");
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert!(
            w.find("environment-editor").bounds().size.height < w.rem_size() * 18.,
            "one variable does not reserve a six-row empty area"
        );
    })
    .expect("compact editor");
    click_label(&app, cx, "Reveal value");
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        let texts = gpui_kit::base::test_support::texts(w);
        assert!(
            texts.iter().any(|t| t.text() == value),
            "literal multiline value is rendered"
        );
        let copy = w
            .elements()
            .into_iter()
            .find(|e| e.description() == Some("Copy value"))
            .expect("copy button");
        assert!(copy.bounds().size.height > gpui_kit::px(0.));
    })
    .expect("reveal");
    assert!(cx.read(|cx| {
        lib.read(cx)
            .vault_item(id)
            .expect("item")
            .used_at()
            .is_none()
    }));
    // Click the real per-value copy, using the observed component identity.
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        let copy = w
            .elements()
            .into_iter()
            .find(|e| e.description() == Some("Copy value"))
            .expect("copy button");
        let id = copy.path().last().expect("id").clone();
        w.click(id, cx);
    })
    .expect("copy");
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| cx.read_from_clipboard().and_then(|c| c.text()))
            .as_deref(),
        Some(value)
    );
    assert!(cx.read(|cx| {
        lib.read(cx)
            .vault_item(id)
            .expect("item")
            .used_at()
            .is_some()
    }));
    let fresh = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let mut fields = l.vault_item(id).expect("item").fields().clone();
            fields.title = "All variables".into();
            l.create_vault_item(&fields, cx).expect("fresh item")
        })
    });
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        passwords.update(cx, |p, cx| p.select_for_dev("All variables", w, cx))
    })
    .expect("select");
    dispatch(&app, cx, CopyProjectEnvironment);
    assert!(cx.read(|cx| {
        lib.read(cx)
            .vault_item(fresh)
            .expect("item")
            .used_at()
            .is_some()
    }));
}

#[gpui_kit::test]
fn environment_export_rejects_existing_files_and_late_locked_dialogs(cx: &mut TestAppContext) {
    use ottero_core::vault::environment::EnvironmentSecrets;
    use ottero_core::vault::{ItemFields, ItemKind};
    use ottero_desktop::actions::{ExportProjectEnvironment, GoPasswords, LockVault};
    let app = unlocked_vault(cx);
    let lib = library(&app, cx);
    cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.create_vault_item(
                &ItemFields {
                    title: "Export test".into(),
                    environment: Some(EnvironmentSecrets::parse("KEY=private").expect("parse")),
                    ..ItemFields::new(ItemKind::Note)
                },
                cx,
            )
            .expect("item")
        })
    });
    dispatch(&app, cx, GoPasswords);
    let passwords = cx.read(|cx| app.shell.read(cx).passwords().clone());
    cx.update_window(app.window, |_, w, cx| {
        passwords.update(cx, |p, cx| p.select_for_dev("Export test", w, cx))
    })
    .expect("select");
    let path = app._dir.path().join("existing.env");
    std::fs::write(&path, "keep original").expect("existing");
    dispatch(&app, cx, ExportProjectEnvironment);
    click_label(&app, cx, "Choose location…");
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(&path).expect("unchanged"),
        "keep original"
    );
    let error = ottero_desktop::library::Library::write_environment_file(&path, "new")
        .expect_err("no overwrite");
    assert!(error.to_string().contains("already exists"));
    let late = app._dir.path().join("late.env");
    dispatch(&app, cx, ExportProjectEnvironment);
    click_label(&app, cx, "Choose location…");
    dispatch(&app, cx, LockVault);
    cx.simulate_new_path_selection(|_| Some(late.clone()));
    cx.run_until_parked();
    assert!(
        !late.exists(),
        "locking before the dialog returns never starts an export"
    );
}

#[gpui_kit::test]
fn developer_projects_reuse_tasks_and_setup_notes(cx: &mut TestAppContext) {
    use ottero_desktop::actions::{GoDeveloper, NewDeveloperProject};
    let app = launch(cx);
    let lib = library(&app, cx);
    dispatch(&app, cx, NewDeveloperProject);
    assert_eq!(route(&app, cx), Route::Developer);
    type_text(&app, cx, "Ottero website");
    press(&app, cx, "enter");
    let project = cx.read(|cx| {
        lib.read(cx)
            .developer_projects(false)
            .expect("projects")
            .remove(0)
    });
    let note_id = project.setup_note().expect("setup note");
    assert_eq!(project.project().name(), "Ottero website");
    click(&app, cx, "developer-setup-note");
    assert_eq!(route(&app, cx), Route::Notes);
    assert!(cx.read(|cx| lib.read(cx).note(note_id).expect("read").is_some()));
    dispatch(&app, cx, GoDeveloper);
    click(&app, cx, "developer-tasks");
    assert_eq!(route(&app, cx), Route::Tasks);
    let tasks = cx.read(|cx| app.shell.read(cx).tasks().clone());
    assert_eq!(cx.read(|cx| tasks.read(cx).project()), Some(project.id()));
    dispatch(&app, cx, GoDeveloper);
    click(&app, cx, "developer-project-menu");
    click_label(&app, cx, "Edit project…");
    click(&app, cx, "developer-name");
    press(&app, cx, leak(secondary("a")));
    type_text(&app, cx, "New website");
    click(&app, cx, "developer-folder");
    type_text(&app, cx, "/tmp/website");
    click(&app, cx, "developer-save");
    let updated = cx.read(|cx| {
        lib.read(cx)
            .developer_project(project.id())
            .expect("read")
            .expect("project")
    });
    assert_eq!(updated.project().name(), "New website");
    assert_eq!(updated.folder(), "/tmp/website");
    cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.archive_developer_project(project.id(), true, cx)
                .expect("archive")
        })
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| {
        lib.read(cx)
            .developer_projects(false)
            .expect("active")
            .is_empty()
    }));
    assert!(cx.read(|cx| lib.read(cx).note(note_id).expect("note").is_some()));
    cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.archive_developer_project(project.id(), false, cx)
                .expect("restore")
        })
    });
    dispatch(&app, cx, GoDeveloper);
    let developer = cx.read(|cx| app.shell.read(cx).developer().clone());
    assert_eq!(
        cx.read(|cx| developer.read(cx).selected()),
        Some(project.id())
    );
    for action in [
        Box::new(UseLightTheme) as Box<dyn gpui_kit::Action>,
        Box::new(UseDarkTheme),
    ] {
        cx.update_window(app.window, |_, w, cx| w.dispatch_action(action, cx))
            .expect("window");
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn developer_secrets_import_edit_export_and_lock(cx: &mut TestAppContext) {
    use ottero_desktop::actions::{
        CopyProjectEnvironment, ExportProjectEnvironment, GoDeveloper, GoPasswords,
        ImportProjectEnvironment, LockVault, NewProjectSecrets,
    };
    let app = unlocked_vault(cx);
    let lib = library(&app, cx);
    let project = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.create_developer_project("Ottero", None, cx)
                .expect("project")
        })
    });
    cx.run_until_parked();
    dispatch(&app, cx, GoDeveloper);
    dispatch(&app, cx, NewProjectSecrets);
    let developer = cx.read(|cx| app.shell.read(cx).developer().clone());
    let pane = cx.read(|cx| developer.read(cx).pane().clone());
    let editor = cx.read(|cx| pane.read(cx).environment_editor().clone());
    let source = (0..80)
        .map(|i| format!("K{i}=unique-value-{i}\n"))
        .collect::<String>()
        + "MULTILINE='line one\nline two'\n";
    let path = app._dir.path().join("import.env");
    std::fs::write(&path, &source).expect("source");
    dispatch(&app, cx, ImportProjectEnvironment);
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| Some(vec![path.clone()]));
    cx.run_until_parked();
    assert!(
        cx.read(|cx| editor.read(cx).document(cx).is_err()),
        "preview must be acknowledged"
    );
    click(&app, cx, "env-apply-import");
    let expected = cx.read(|cx| editor.read(cx).document(cx).expect("document"));
    assert_eq!(expected.variables().len(), 81);
    assert_eq!(expected.variables()[80].value(), "line one\nline two");
    assert_eq!(expected.project_id(), Some(project));
    cx.update_window(app.window, |_, w, cx| {
        pane.update(cx, |p, cx| p.save(w, cx))
    })
    .expect("save");
    cx.run_until_parked();
    let id = cx.read(|cx| pane.read(cx).item_id().expect("saved"));
    assert_eq!(
        cx.read(|cx| lib
            .read(cx)
            .vault_item(id)
            .expect("item")
            .fields()
            .environment
            .clone()),
        Some(expected.clone())
    );
    let out = app._dir.path().join("export.env");
    dispatch(&app, cx, CopyProjectEnvironment);
    let copied = cx.read(|cx| {
        cx.read_from_clipboard()
            .and_then(|c| c.text())
            .expect("copied")
    });
    assert_eq!(
        ottero_core::vault::environment::EnvironmentSecrets::parse(&copied)
            .expect("copy")
            .variables(),
        expected.variables()
    );
    dispatch(&app, cx, ExportProjectEnvironment);
    assert!(dialog_open(&app, cx));
    click_label(&app, cx, "Choose location…");
    cx.simulate_new_path_selection(|_| Some(out.clone()));
    cx.run_until_parked();
    assert!(
        cx.read(|cx| lib.read(cx).export_environment(id, &out).is_err()),
        "never overwrite"
    );
    let exported = std::fs::read_to_string(&out).expect("read");
    let parsed =
        ottero_core::vault::environment::EnvironmentSecrets::parse(&exported).expect("parse");
    assert_eq!(parsed.variables(), expected.variables());
    cx.executor().advance_clock(Duration::from_secs(31));
    cx.run_until_parked();
    assert!(
        cx.read(|cx| cx
            .read_from_clipboard()
            .and_then(|c| c.text())
            .unwrap_or_default()
            .is_empty()),
        "secret clipboard clears"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&out)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    dispatch(&app, cx, GoPasswords);
    let passwords = cx.read(|cx| app.shell.read(cx).passwords().clone());
    let other_pane = cx.read(|cx| passwords.read(cx).pane().clone());
    cx.update_window(app.window, |_, w, cx| {
        other_pane.update(cx, |p, cx| {
            p.show(Some(id), w, cx);
            p.edit(w, cx);
            p.save(w, cx);
        })
    })
    .expect("edit through Passwords");
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| lib
            .read(cx)
            .vault_item(id)
            .expect("item")
            .fields()
            .environment
            .clone()),
        Some(expected)
    );
    dispatch(&app, cx, GoDeveloper);
    cx.update_window(app.window, |_, w, cx| {
        pane.update(cx, |p, cx| {
            p.show(Some(id), w, cx);
            p.edit(w, cx);
        })
    })
    .expect("edit");
    cx.update_window(app.window, |_, w, cx| {
        editor.update(cx, |s, cx| s.import(w, cx))
    })
    .expect("pending import");
    dispatch(&app, cx, LockVault);
    cx.simulate_path_prompt_response(|_| Some(vec![path]));
    cx.run_until_parked();
    assert!(
        cx.read(|cx| pane.read(cx).draft_is_empty(cx)),
        "lock clears every secret editor"
    );
    assert!(
        cx.read(|cx| editor.read(cx).draft_is_empty(cx)),
        "late import cannot refill locked draft"
    );
    assert!(cx.read(|cx| {
        lib.read(cx)
            .export_environment(id, &app._dir.path().join("locked.env"))
            .is_err()
    }));
}

#[gpui_kit::test]
fn developer_late_import_cannot_replace_another_draft(cx: &mut TestAppContext) {
    use ottero_desktop::actions::{GoDeveloper, ImportProjectEnvironment, NewProjectSecrets};
    let app = unlocked_vault(cx);
    let lib = library(&app, cx);
    let (first, second) = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            (
                l.create_developer_project("First", None, cx)
                    .expect("first"),
                l.create_developer_project("Second", None, cx)
                    .expect("second"),
            )
        })
    });
    dispatch(&app, cx, GoDeveloper);
    let developer = cx.read(|cx| app.shell.read(cx).developer().clone());
    cx.update_window(app.window, |_, w, cx| {
        developer.update(cx, |d, cx| d.select(first, w, cx))
    })
    .expect("select");
    dispatch(&app, cx, NewProjectSecrets);
    let pane = cx.read(|cx| developer.read(cx).pane().clone());
    let editor = cx.read(|cx| pane.read(cx).environment_editor().clone());
    let path = app._dir.path().join("late.env");
    std::fs::write(&path, "OLD_DRAFT=must-not-appear").expect("file");
    dispatch(&app, cx, ImportProjectEnvironment);
    // Discard the first draft and start another while its file picker is open.
    cx.update_window(app.window, |_, w, cx| {
        pane.update(cx, |p, cx| {
            p.cancel(w, cx);
            p.start_environment(second, "Second", w, cx);
        })
    })
    .expect("replace draft");
    cx.simulate_path_prompt_response(|_| Some(vec![path]));
    cx.run_until_parked();
    let doc = cx.read(|cx| editor.read(cx).document(cx).expect("no stale preview"));
    assert_eq!(doc.project_id(), Some(second));
    assert!(doc.variables().is_empty());
}

#[gpui_kit::test]
fn locking_wipes_unsaved_drafts_and_closes_the_generator(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt as _;
    use ottero_core::vault::ItemKind;
    use ottero_core::vault::lock::LockState;
    use ottero_desktop::actions::{GeneratePassword, LockVault};

    let app = unlocked_vault(cx);
    let lib = library(&app, cx);
    let passwords = cx.read(|cx| app.shell.read(cx).passwords().clone());
    let pane = cx.read(|cx| passwords.read(cx).pane().clone());
    cx.update_window(app.window, |_, window, cx| {
        pane.update(cx, |p, cx| {
            p.start_new(ItemKind::Login, window, cx);
            p.type_draft_password_for_tests("half-typed-secret", window, cx);
        });
    })
    .expect("window");
    cx.run_until_parked();
    assert!(!cx.read(|cx| pane.read(cx).draft_is_empty(cx)));

    dispatch(&app, cx, GeneratePassword);
    let open = cx
        .update_window(app.window, |_, window, cx| window.has_active_dialog(cx))
        .expect("window");
    assert!(open, "generator open");

    dispatch(&app, cx, LockVault);
    assert_eq!(cx.read(|cx| lib.read(cx).vault_state()), LockState::Locked);
    assert!(
        cx.read(|cx| pane.read(cx).draft_is_empty(cx)),
        "draft wiped"
    );
    assert!(!cx.read(|cx| pane.read(cx).is_editing()));
    let open = cx
        .update_window(app.window, |_, window, cx| window.has_active_dialog(cx))
        .expect("window");
    assert!(!open, "generator closed on lock");
}

#[gpui_kit::test]
fn waking_from_sleep_locks_the_vault(cx: &mut TestAppContext) {
    use ottero_core::vault::lock::LockState;

    let app = unlocked_vault(cx);
    let lib = library(&app, cx);
    let passwords = cx.read(|cx| app.shell.read(cx).passwords().clone());
    cx.update(|cx| passwords.update(cx, |p, _| p.pretend_slept_for_tests()));
    cx.executor().advance_clock(Duration::from_secs(11));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| lib.read(cx).vault_state()), LockState::Locked);
}

fn set_up_with_key(app: &App, cx: &mut TestAppContext) -> String {
    use ottero_desktop::actions::GoPasswords;
    let lib = library(app, cx);
    cx.update(|cx| {
        lib.update(cx, |l, _| {
            l.set_kdf_params_for_tests(ottero_crypto::KdfParams::MINIMUM)
        })
    });
    dispatch(app, cx, GoPasswords);
    let setup = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.set_up_vault(
                ottero_crypto::SecretString::from("plum-orbit-candle-9-meadow".to_owned()),
                cx,
            )
        })
    });
    let key = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let out = key.clone();
    cx.foreground_executor()
        .spawn(async move {
            if let Ok(k) = setup.await {
                *out.borrow_mut() = k.to_string();
            }
        })
        .detach();
    cx.run_until_parked();
    let k = key.borrow().clone();
    assert!(!k.is_empty(), "setup finished");
    k
}

#[gpui_kit::test]
fn a_forgotten_password_is_recovered_with_the_recovery_key(cx: &mut TestAppContext) {
    use ottero_core::vault::lock::LockState;
    use ottero_core::vault::{ItemFields, ItemKind};
    use ottero_desktop::actions::LockVault;

    let app = launch(cx);
    let lib = library(&app, cx);
    let key = set_up_with_key(&app, cx);
    let id = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let f = ItemFields {
                title: "Bank".into(),
                password: "keep-me-1".into(),
                ..ItemFields::new(ItemKind::Login)
            };
            l.create_vault_item(&f, cx).expect("item")
        })
    });
    dispatch(&app, cx, LockVault);
    let wrong = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.recover_vault(
                "AAAA-AAAA-AAAA-AAAA-AAAA-AAAA-AAAA-AAAA",
                Some(ottero_crypto::SecretString::from(
                    "a whole new master 7".to_owned(),
                )),
                cx,
            )
        })
    });
    wrong.detach();
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| lib.read(cx).vault_state()), LockState::Locked);
    // Typed loosely: lower case, spaces instead of dashes.
    let typed = key.to_lowercase().replace('-', " ");
    let right = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.recover_vault(
                &typed,
                Some(ottero_crypto::SecretString::from(
                    "a whole new master 7".to_owned(),
                )),
                cx,
            )
        })
    });
    right.detach();
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| lib.read(cx).vault_state()),
        LockState::Unlocked
    );
    assert!(
        cx.read(|cx| lib.read(cx).vault_item(id).is_some()),
        "nothing lost"
    );
}

#[gpui_kit::test]
fn starting_over_archives_the_old_vault_and_it_can_be_reopened(cx: &mut TestAppContext) {
    use ottero_core::vault::lock::LockState;
    use ottero_core::vault::{ItemFields, ItemKind};
    use ottero_desktop::actions::LockVault;

    let app = launch(cx);
    let lib = library(&app, cx);
    let dir = app.paths.data_dir.join("vault-archives");
    cx.update(|cx| lib.update(cx, |l, _| l.set_archive_dir(dir)));
    set_up_with_key(&app, cx);
    cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let f = ItemFields {
                title: "Old bank".into(),
                password: "pw".into(),
                ..ItemFields::new(ItemKind::Login)
            };
            l.create_vault_item(&f, cx).expect("item")
        })
    });
    dispatch(&app, cx, LockVault);
    let path = cx
        .update(|cx| lib.update(cx, |l, cx| l.start_new_vault(cx)))
        .expect("archive");
    assert!(path.exists());
    assert_eq!(
        cx.read(|cx| lib.read(cx).vault_state()),
        LockState::Uninitialized
    );
    assert_eq!(cx.read(|cx| lib.read(cx).vault_archives().len()), 1);
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("otvault"));
    // Archives written before the renames (same format) are still listed.
    for old in ["cfvault", "ksvault"] {
        std::fs::copy(&path, path.with_extension(old)).expect("copy");
    }
    assert_eq!(cx.read(|cx| lib.read(cx).vault_archives().len()), 3);

    // New vault, then the old password brings the old items back.
    set_up_with_key(&app, cx);
    let open = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.import_vault_archive(
                path.clone(),
                ottero_desktop::library::ArchiveSecret::Password(
                    ottero_crypto::SecretString::from("plum-orbit-candle-9-meadow".to_owned()),
                ),
                cx,
            )
        })
    });
    open.detach();
    cx.run_until_parked();
    let titles: Vec<String> = cx.read(|cx| {
        lib.read(cx)
            .vault()
            .items()
            .iter()
            .map(|i| i.title().to_owned())
            .collect()
    });
    assert_eq!(titles, ["Old bank"]);
}

#[gpui_kit::test]
fn encrypted_backup_exports_and_restores_without_duplicates(cx: &mut TestAppContext) {
    use ottero_core::vault::{ItemFields, ItemKind};
    use ottero_desktop::actions::LockVault;

    let app = launch(cx);
    let lib = library(&app, cx);
    set_up_with_key(&app, cx);
    let make = |title: &str| ItemFields {
        title: title.into(),
        password: format!("{title}-secret-pw"),
        ..ItemFields::new(ItemKind::Login)
    };
    let (bank, mail) = (make("Bank"), make("Mail"));
    let bank_id = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.create_vault_item(&mail, cx).expect("mail");
            l.create_vault_item(&bank, cx).expect("bank")
        })
    });
    let path = app.paths.data_dir.join("backups/Ottero Vault.otvault");
    cx.read(|cx| lib.read(cx).export_vault(&path))
        .expect("export");
    let text = std::fs::read_to_string(&path).expect("file");
    assert!(
        !text.contains("Bank") && !text.contains("secret-pw"),
        "the backup stays encrypted"
    );

    // Lose one item, then restore: it comes back and Mail isn't doubled.
    cx.update(|cx| lib.update(cx, |l, cx| l.delete_vault_items(&[bank_id], cx)))
        .expect("delete");
    let restore = |cx: &mut TestAppContext, password: &str| {
        let task = cx.update(|cx| {
            lib.update(cx, |l, cx| {
                l.import_vault_archive(
                    path.clone(),
                    ottero_desktop::library::ArchiveSecret::Password(
                        ottero_crypto::SecretString::from(password.to_owned()),
                    ),
                    cx,
                )
            })
        });
        let result = std::rc::Rc::new(std::cell::RefCell::new(None));
        let out = result.clone();
        cx.spawn(async move |_| *out.borrow_mut() = Some(task.await.expect("restore")))
            .detach();
        cx.run_until_parked();
        result.borrow_mut().take().expect("finished")
    };
    assert_eq!(restore(cx, "not the password"), None);
    assert_eq!(restore(cx, "plum-orbit-candle-9-meadow"), Some(1));
    assert_eq!(restore(cx, "plum-orbit-candle-9-meadow"), Some(0));
    let mut titles: Vec<String> = cx.read(|cx| {
        lib.read(cx)
            .vault()
            .items()
            .iter()
            .map(|i| i.title().to_owned())
            .collect()
    });
    titles.sort();
    assert_eq!(titles, ["Bank", "Mail"]);

    dispatch(&app, cx, LockVault);
    assert!(
        cx.read(|cx| lib.read(cx).export_vault(&path)).is_err(),
        "export needs an unlocked vault"
    );
}

#[gpui_kit::test]
fn one_occurrence_of_a_repeating_event_can_change_on_its_own(cx: &mut TestAppContext) {
    use ottero_core::{Recurrence, Time};
    use ottero_desktop::actions::GoCalendar;
    use ottero_desktop::views::event_detail::open_event_detail;

    let app = launch(cx);
    dispatch(&app, cx, GoCalendar);
    let lib = library(&app, cx);
    let calendar = cx.read(|cx| app.shell.read(cx).calendar().clone());
    let today = ottero_core::clock::today();
    let tomorrow = today.tomorrow().expect("date");
    let id = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let id = l
                .create_timed_event(
                    "Walk",
                    today.to_datetime(Time::constant(7, 0, 0, 0)),
                    today.to_datetime(Time::constant(7, 30, 0, 0)),
                    cx,
                )
                .expect("event");
            l.set_event_recurrence(id, Some(Recurrence::DAILY), cx)
                .expect("repeat");
            id
        })
    });
    let walks_on = |cx: &mut TestAppContext, day| {
        cx.update_window(app.window, |_, window, _| window.refresh())
            .expect("window");
        cx.run_until_parked();
        cx.read(|cx| {
            calendar
                .read(cx)
                .span()
                .on_day(day)
                .filter(|o| o.event == id)
                .count()
        })
    };
    assert_eq!(walks_on(cx, tomorrow), 1);

    // Delete only tomorrow's walk from its panel: the question comes first.
    let open = |cx: &mut TestAppContext| {
        let lib = lib.clone();
        cx.update_window(app.window, |_, window, cx| {
            open_event_detail(id, Some(tomorrow), false, lib, window, cx);
        })
        .expect("window");
        cx.run_until_parked();
    };
    open(cx);
    click(&app, cx, "event-delete");
    assert!(dialog_open(&app, cx), "asks this one or all");
    click(&app, cx, "scope-this");
    assert_eq!(walks_on(cx, tomorrow), 0);
    assert_eq!(walks_on(cx, today), 1, "the rest stay");
    let event = cx.read(|cx| lib.read(cx).event(id)).expect("event");
    assert!(!event.is_trashed());
    cx.update(|cx| {
        lib.update(cx, |l, cx| l.restore_occurrence(id, tomorrow, None, cx))
            .expect("undo")
    });
    assert_eq!(walks_on(cx, tomorrow), 1);

    // Dragging one occurrence moves just it (a day later, same time).
    let later = tomorrow.tomorrow().expect("date");
    let previous = cx
        .update(|cx| {
            lib.update(cx, |l, cx| {
                l.move_occurrence_to_day(id, tomorrow, later, cx)
            })
        })
        .expect("move");
    assert_eq!(previous, None);
    assert_eq!(walks_on(cx, tomorrow), 0);
    assert_eq!(walks_on(cx, later), 2, "its own walk plus the moved one");
    let moved = cx
        .read(|cx| lib.read(cx).event(id))
        .and_then(|e| e.occurrence_on(tomorrow))
        .expect("moved");
    assert!(moved.moved);
    assert_eq!(moved.start, later.to_datetime(Time::constant(7, 0, 0, 0)));

    // Cancel does nothing; "Delete all events" deletes the series.
    open(cx);
    click(&app, cx, "event-delete");
    click(&app, cx, "scope-cancel");
    assert!(
        !cx.read(|cx| lib.read(cx).event(id))
            .expect("e")
            .is_trashed()
    );
    click(&app, cx, "event-delete");
    click(&app, cx, "scope-all");
    assert!(
        cx.read(|cx| lib.read(cx).event(id))
            .expect("e")
            .is_trashed()
    );
    assert_eq!(walks_on(cx, today), 0);
}

#[gpui_kit::test]
fn deleted_events_can_be_restored_or_removed_for_good(cx: &mut TestAppContext) {
    use ottero_core::Time;
    use ottero_desktop::actions::{GoCalendar, ShowDeletedEvents};

    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    dispatch(&app, cx, GoCalendar);
    let lib = library(&app, cx);
    let today = ottero_core::clock::today();
    let make = |cx: &mut TestAppContext, title: &str| {
        cx.update(|cx| {
            lib.update(cx, |l, cx| {
                let id = l
                    .create_timed_event(
                        title,
                        today.to_datetime(Time::constant(15, 0, 0, 0)),
                        today.to_datetime(Time::constant(16, 0, 0, 0)),
                        cx,
                    )
                    .expect("event");
                l.trash_event(id, cx).expect("trash");
                id
            })
        })
    };
    let dentist = make(cx, "Dentist");
    let party = make(cx, "Party");
    let trashed = |cx: &mut TestAppContext, id| {
        cx.read(|cx| lib.read(cx).event(id))
            .map(|e: ottero_core::Event| e.is_trashed())
    };

    dispatch(&app, cx, ShowDeletedEvents);
    click(&app, cx, leak(format!("restore-event-{dentist}")));
    assert_eq!(trashed(cx, dentist), Some(false), "restored");

    click(&app, cx, leak(format!("forget-event-{party}")));
    assert!(dialog_open(&app, cx), "permanent delete asks first");
    press(&app, cx, "escape");
    assert_eq!(trashed(cx, party), Some(true), "Escape keeps it");
    click(&app, cx, leak(format!("forget-event-{party}")));
    press(&app, cx, "enter");
    assert_eq!(trashed(cx, party), None, "gone for good");

    // Empty trash removes everything that's left, after confirming.
    cx.update(|cx| lib.update(cx, |l, cx| l.trash_event(dentist, cx)))
        .expect("trash");
    click(&app, cx, "empty-event-trash");
    press(&app, cx, "enter");
    assert_eq!(trashed(cx, dentist), None);
    assert!(
        cx.read(|cx| lib.read(cx).deleted_events())
            .expect("list")
            .is_empty()
    );
}

#[gpui_kit::test]
fn ics_files_import_once(cx: &mut TestAppContext) {
    use ottero_desktop::actions::GoCalendar;

    let app = launch(cx);
    dispatch(&app, cx, GoCalendar);
    let lib = library(&app, cx);
    let calendar = cx.read(|cx| app.shell.read(cx).calendar().clone());
    let today = ottero_core::clock::today();
    let day = today.strftime("%Y%m%d").to_string();
    let text = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Imported lunch\r\n\
         DTSTART:{day}T130000\r\nDTEND:{day}T140000\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Gym\r\nDTSTART:{day}T070000\r\nDURATION:PT1H\r\n\
         RRULE:FREQ=DAILY;COUNT=3\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    let first = cx
        .update(|cx| lib.update(cx, |l, cx| l.import_ics(&text, cx)))
        .expect("import");
    assert_eq!((first.added, first.duplicates), (2, 0));
    assert_eq!(first.message(), "Imported 2 events.");
    let again = cx
        .update(|cx| lib.update(cx, |l, cx| l.import_ics(&text, cx)))
        .expect("import");
    assert_eq!((again.added, again.duplicates), (0, 2));
    assert!(again.message().starts_with("Nothing new to import."));

    cx.update_window(app.window, |_, window, _| window.refresh())
        .expect("window");
    cx.run_until_parked();
    let titles: Vec<String> = cx.read(|cx| {
        let span = calendar.read(cx).span().clone();
        span.on_day(today)
            .filter_map(|o| span.events.get(&o.event).map(|e| e.title().to_owned()))
            .collect()
    });
    assert_eq!(titles, ["Gym", "Imported lunch"]);
    let gym = cx.read(|cx| {
        let span = calendar.read(cx).span().clone();
        span.events.values().find(|e| e.title() == "Gym").cloned()
    });
    assert_eq!(
        gym.and_then(|g| g.until()),
        today.checked_add(jiff::Span::new().days(2)).ok(),
        "COUNT=3 ends two days later"
    );
    assert!(
        cx.update(|cx| lib.update(cx, |l, cx| l.import_ics("not a calendar", cx)))
            .is_err()
    );
}

#[gpui_kit::test]
fn holidays_show_quietly_follow_settings_and_travel(cx: &mut TestAppContext) {
    use ottero_desktop::actions::GoCalendar;
    use ottero_desktop::views::calendar_layout::CalendarMode;
    use ottero_ui::appearance;

    let app = launch(cx);
    cx.update(|cx| {
        ottero_desktop::holidays::set_detected_country_for_tests(Some("IN"), cx);
        appearance::update(cx, |s| {
            s.holidays = true;
            s.holiday_observances = true;
            s.holidays_where_you_are = true;
            s.holiday_countries = Some(vec!["IN".into()]);
        });
    });
    dispatch(&app, cx, GoCalendar);
    let calendar = cx.read(|cx| app.shell.read(cx).calendar().clone());
    let diwali: ottero_core::Date = "2026-11-08".parse().expect("date");
    let thanksgiving: ottero_core::Date = "2026-11-26".parse().expect("date");
    let names = |cx: &mut TestAppContext, day| -> Vec<String> {
        cx.update_window(app.window, |_, window, _| window.refresh())
            .expect("window");
        cx.run_until_parked();
        cx.read(|cx| {
            let c = calendar.read(cx);
            c.holidays()
                .on(day)
                .iter()
                .map(|h| c.holidays().label(h))
                .collect()
        })
    };
    cx.update(|cx| calendar.update(cx, |c, cx| c.select(diwali, cx)));
    for (mode, _) in CalendarMode::OPTIONS {
        cx.update(|cx| calendar.update(cx, |c, cx| c.set_mode(mode, cx)));
        assert_eq!(
            names(cx, diwali)[0],
            "Diwali",
            "public holiday first, {mode:?}"
        );
    }
    cx.update(|cx| calendar.update(cx, |c, cx| c.set_mode(CalendarMode::Month, cx)));
    assert!(names(cx, thanksgiving).is_empty(), "only India at home");

    // Travelling to the US: its holidays join, labelled with the country.
    cx.update(|cx| {
        ottero_desktop::holidays::set_detected_country_for_tests(Some("US"), cx);
        appearance::update(cx, |s| s.holidays_where_you_are = true);
        calendar.update(cx, |c, cx| c.reload(cx));
    });
    assert_eq!(names(cx, thanksgiving), ["Thanksgiving Day · US"]);
    assert_eq!(names(cx, diwali)[0], "Diwali · IN");
    cx.update(|cx| appearance::update(cx, |s| s.holidays_where_you_are = false));
    assert!(
        names(cx, thanksgiving).is_empty(),
        "travel labels can be turned off"
    );

    // A state adds its own days: Pongal is a day off in Tamil Nadu.
    let pongal: ottero_core::Date = "2026-01-14".parse().expect("date");
    cx.update(|cx| {
        appearance::update(cx, |s| s.holiday_observances = false);
        calendar.update(cx, |c, cx| c.select(pongal, cx));
    });
    assert!(!names(cx, pongal).iter().any(|n| n.starts_with("Pongal")));
    cx.update(|cx| appearance::update(cx, |s| s.holiday_regions = vec!["IN-TN".into()]));
    assert!(
        names(cx, pongal).contains(&"Pongal · Tamil Nadu".to_owned()),
        "{:?}",
        names(cx, pongal)
    );
    cx.update(|cx| appearance::update(cx, |s| s.holiday_observances = true));

    // Observances (Raksha Bandhan in India) and the whole layer can be switched off.
    let rakhi: ottero_core::Date = "2026-08-28".parse().expect("date");
    cx.update(|cx| calendar.update(cx, |c, cx| c.select(rakhi, cx)));
    assert_eq!(names(cx, rakhi), ["Raksha Bandhan"]);
    cx.update(|cx| appearance::update(cx, |s| s.holiday_observances = false));
    assert!(names(cx, rakhi).is_empty());
    cx.update(|cx| calendar.update(cx, |c, cx| c.select(diwali, cx)));
    cx.update(|cx| appearance::update(cx, |s| s.holidays = false));
    assert!(names(cx, diwali).is_empty());
}

#[gpui_kit::test]
fn birthdays_come_from_quick_add_and_can_be_hidden(cx: &mut TestAppContext) {
    use ottero_core::OccasionKind;
    use ottero_desktop::actions::GoCalendar;
    use ottero_ui::appearance;

    let app = launch(cx);
    dispatch(&app, cx, GoCalendar);
    let lib = library(&app, cx);
    let calendar = cx.read(|cx| app.shell.read(cx).calendar().clone());
    let today = ottero_core::clock::today();
    let born = today.year() - 60;
    let id = cx
        .update(|cx| {
            lib.update(cx, |l, cx| {
                l.quick_add_event(&format!("Mom's birthday {born}"), today, cx)
            })
        })
        .expect("add");
    let event = cx.read(|cx| lib.read(cx).event(id)).expect("event");
    let occasion = event.occasion().expect("occasion");
    assert_eq!(
        (occasion.kind, occasion.year),
        (OccasionKind::Birthday, Some(born))
    );
    assert!(event.is_all_day());
    assert_eq!(occasion.note(today).as_deref(), Some("turns 60"));

    let shown = |cx: &mut TestAppContext| {
        cx.update_window(app.window, |_, window, _| window.refresh())
            .expect("window");
        cx.run_until_parked();
        cx.read(|cx| {
            let c = calendar.read(cx);
            c.span()
                .on_day(today)
                .filter(|o| o.event == id)
                .any(|o| c.shows_for_test(o, cx))
        })
    };
    assert!(shown(cx));
    cx.update(|cx| appearance::update(cx, |s| s.calendar_occasions = false));
    assert!(!shown(cx), "View options can hide birthdays");
    cx.update(|cx| appearance::update(cx, |s| s.calendar_occasions = true));

    // Back to a plain event from the panel's Type control.
    cx.update(|cx| lib.update(cx, |l, cx| l.set_event_occasion(id, None, cx)))
        .expect("plain");
    assert_eq!(
        cx.read(|cx| lib.read(cx).event(id))
            .and_then(|e| e.occasion()),
        None
    );
}

#[gpui_kit::test]
fn templates_custom_fields_tags_and_search(cx: &mut TestAppContext) {
    use ottero_core::vault::template::Template;
    use ottero_core::vault::{FieldKind, ItemFields, ItemKind};

    let app = unlocked_vault(cx);
    let lib = library(&app, cx);
    let passwords = cx.read(|cx| app.shell.read(cx).passwords().clone());
    let pane = cx.read(|cx| passwords.read(cx).pane().clone());
    cx.update_window(app.window, |_, window, cx| {
        pane.update(cx, |p, cx| p.start_template(Template::Wifi, window, cx));
    })
    .expect("window");
    type_text(&app, cx, "Home Wi-Fi");
    click(&app, cx, "edit-save");
    cx.run_until_parked();
    let wifi = cx
        .read(|cx| {
            lib.read(cx)
                .vault()
                .items()
                .iter()
                .find(|i| i.title() == "Home Wi-Fi")
                .cloned()
        })
        .expect("saved");
    assert_eq!(wifi.kind(), ItemKind::Note);
    assert_eq!(wifi.fields().template, Some(Template::Wifi));
    assert!(
        wifi.fields()
            .custom
            .iter()
            .any(|c| c.kind == FieldKind::Secret && c.label == "Password"),
        "suggested fields stay, ready to fill"
    );

    cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let f = ItemFields {
                title: "GitHub".into(),
                tags: vec!["work".into()],
                custom: vec![ottero_core::vault::CustomField {
                    kind: FieldKind::Text,
                    label: "Account ID".into(),
                    value: "acme-42".into(),
                }],
                ..ItemFields::new(ItemKind::Login)
            };
            l.create_vault_item(&f, cx).expect("item")
        })
    });
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| lib.read(cx).vault_tags()),
        [("work".to_owned(), 1)]
    );
    cx.update_window(app.window, |_, window, cx| {
        passwords.update(cx, |p, cx| p.set_tag(Some("work".into()), window, cx));
    })
    .expect("window");
    assert_eq!(cx.read(|cx| passwords.read(cx).ids().len()), 1);
    let found = cx.update(|cx| {
        lib.update(cx, |l, _| {
            (
                l.vault_list(Default::default(), None, "githb", Default::default())
                    .len(),
                l.vault_list(Default::default(), None, "type:note", Default::default())
                    .len(),
                l.vault_list(Default::default(), None, "acme", Default::default())
                    .len(),
            )
        })
    });
    assert_eq!(found, (1, 1, 1), "typo, filter word, custom field value");
}

#[gpui_kit::test]
fn focus_task_picker_searches_beyond_the_first_page(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let lib = library(&app, cx);
    let needle = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let id = l
                .create_task("Needle deep in the task list", None, cx)
                .expect("task");
            for ix in 0..300 {
                l.create_task(&format!("Other task {ix}"), None, cx)
                    .expect("task");
            }
            id
        })
    });
    dispatch(&app, cx, GoFocus);
    click(&app, cx, "focus-task-picker");
    type_text(&app, cx, "NEEDLE");
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    press(&app, cx, "enter");
    assert_eq!(
        cx.read(|cx| app.shell.read(cx).focus_ctrl().read(cx).timer().task_id),
        Some(needle)
    );
    click(&app, cx, "focus-clear-task");
    assert_eq!(
        cx.read(|cx| app.shell.read(cx).focus_ctrl().read(cx).timer().task_id),
        None
    );
}

#[gpui_kit::test]
fn focus_duration_menu_validates_custom_minutes(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    dispatch(&app, cx, GoFocus);
    click(&app, cx, "focus-duration");
    click_label(&app, cx, "50 minutes");
    assert_eq!(
        cx.read(|cx| app
            .shell
            .read(cx)
            .focus_ctrl()
            .read(cx)
            .timer()
            .planned_secs),
        50 * 60
    );
    click(&app, cx, "focus-duration");
    click_label(&app, cx, "Custom duration…");
    press(
        &app,
        cx,
        if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        },
    );
    type_text(&app, cx, "240");
    press(&app, cx, "enter");
    assert_eq!(
        cx.read(|cx| app
            .shell
            .read(cx)
            .focus_ctrl()
            .read(cx)
            .timer()
            .planned_secs),
        50 * 60,
        "Out-of-range input must not silently clamp or change the timer"
    );
    press(
        &app,
        cx,
        if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        },
    );
    type_text(&app, cx, "37");
    press(&app, cx, "enter");
    assert_eq!(
        cx.read(|cx| app
            .shell
            .read(cx)
            .focus_ctrl()
            .read(cx)
            .timer()
            .planned_secs),
        37 * 60
    );
}

#[gpui_kit::test]
fn focus_complete_task_undo_restores_in_progress_status(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let lib = library(&app, cx);
    let task_id = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let id = l.create_task("Already working", None, cx).expect("create");
            l.set_task_status(id, TaskStatus::InProgress, cx)
                .expect("start task");
            id
        })
    });
    dispatch(&app, cx, GoFocus);
    cx.update_window(app.window, |_, window, cx| {
        let ctrl = app.shell.read(cx).focus_ctrl().clone();
        ctrl.update(cx, |f, cx| {
            f.start_task_focus(task_id, "Already working", window, cx);
            f.finish_now(true, cx).expect("finish");
        });
    })
    .expect("window");
    cx.run_until_parked();
    click_label(&app, cx, "Mark task done");
    assert_eq!(
        cx.read(|cx| lib.read(cx).tasks_by_id(task_id).expect("task").status()),
        TaskStatus::Done
    );
    click(&app, cx, "undo");
    assert_eq!(
        cx.read(|cx| lib.read(cx).tasks_by_id(task_id).expect("task").status()),
        TaskStatus::InProgress
    );
}

#[gpui_kit::test]
fn focus_failed_save_preserves_progress_and_can_be_retried(cx: &mut TestAppContext) {
    let app = launch(cx);
    dispatch(&app, cx, GoFocus);
    let lib = library(&app, cx);
    // A missing linked task causes a real SQLite foreign-key failure.
    cx.update_window(app.window, |_, window, cx| {
        let controller = app.shell.read(cx).focus_ctrl().clone();
        controller.update(cx, |f, cx| {
            f.start_task_focus(ottero_core::TaskId::new(), "Missing task", window, cx);
            assert!(f.finish_now(true, cx).is_err());
            assert!(f.timer().is_paused());
            assert_eq!(f.timer().cycle_completed, 0);
            assert!(f.save_error().is_some());
            let before = f.timer().clone();
            f.set_kind(ottero_core::FocusKind::ShortBreak, cx);
            assert_eq!(
                f.timer(),
                &before,
                "A mode change must preserve an unsaved session"
            );
            f.attach_task(None, "Recovered session", cx);
            f.finish_now(true, cx).expect("retry save");
            assert!(f.save_error().is_none());
            assert!(f.timer().is_idle());
            assert_eq!(f.timer().cycle_completed, 1);
        });
    })
    .expect("window");
    cx.run_until_parked();
    let sessions = cx.read(|cx| {
        lib.read(cx)
            .focus_sessions_between(ottero_core::Timestamp::MIN, ottero_core::Timestamp::MAX)
            .expect("sessions")
    });
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].label, "Recovered session");
}

#[gpui_kit::test]
fn focus_mode_tabs_preserve_running_and_paused_sessions(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt as _;
    use ottero_core::FocusKind;

    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    dispatch(&app, cx, GoFocus);
    for light in [false, true] {
        if light {
            dispatch(&app, cx, UseLightTheme);
        } else {
            dispatch(&app, cx, UseDarkTheme);
        }
        click_label(&app, cx, "Short break");
        press(&app, cx, "space");
        let timer = cx.read(|cx| app.shell.read(cx).focus_ctrl().read(cx).timer().clone());
        assert_eq!(timer.kind, FocusKind::ShortBreak);
        assert!(timer.is_running());
        for _ in 0..12 {
            click_label(&app, cx, "Long break");
            click_label(&app, cx, "Short break");
        }
        assert_eq!(
            cx.update_window(app.window, |_, window, cx| window.notifications(cx).len())
                .expect("window"),
            0,
            "Repeated mode clicks must never stack notifications"
        );
        assert!(
            cx.update_window(app.window, |_, window, _| {
                window.try_find("focus-mode-hint").is_none()
            })
            .expect("window")
        );
        assert_eq!(
            cx.read(|cx| app.shell.read(cx).focus_ctrl().read(cx).timer().clone()),
            timer,
            "Mode clicks must not reset or cancel a running break"
        );
        press(&app, cx, "space");
        let paused = cx.read(|cx| app.shell.read(cx).focus_ctrl().read(cx).timer().clone());
        assert!(paused.is_paused());
        click_label(&app, cx, "Long break");
        assert_eq!(
            cx.read(|cx| app.shell.read(cx).focus_ctrl().read(cx).timer().clone()),
            paused,
            "A paused session is still protected"
        );
        dispatch(&app, cx, ottero_desktop::actions::FocusReset);
        assert!(
            cx.update_window(app.window, |_, window, _| {
                window.try_find("focus-mode-hint").is_none()
            })
            .expect("window")
        );
        click_label(&app, cx, "Long break");
        assert_eq!(
            cx.read(|cx| app.shell.read(cx).focus_ctrl().read(cx).timer().kind),
            FocusKind::LongBreak,
            "Reset explicitly allows a new mode"
        );
    }
}

#[gpui_kit::test]
fn focus_timer_attaches_to_tasks_persists_and_shows_header_pill_off_screen(
    cx: &mut TestAppContext,
) {
    let app = launch(cx);
    let lib = library(&app, cx);
    let task_id = cx.update(|cx| {
        lib.update(cx, |l, cx| l.create_task("Write release notes", None, cx))
            .expect("create task")
    });

    // Navigate to Focus via action and test Spacebar toggle + task focus.
    dispatch(&app, cx, GoFocus);
    assert_eq!(route(&app, cx), Route::Focus);
    press(&app, cx, "space");
    assert!(
        cx.read(|cx| app
            .shell
            .read(cx)
            .focus_ctrl()
            .read(cx)
            .timer()
            .is_running()),
        "Space on Focus screen should start the timer once"
    );
    press(&app, cx, "space");
    assert!(
        cx.read(|cx| app.shell.read(cx).focus_ctrl().read(cx).timer().is_paused()),
        "Space again on Focus screen should pause the timer once"
    );

    cx.update_window(app.window, |_, window, cx| {
        app.shell.update(cx, |s, cx| {
            s.focus_ctrl().update(cx, |f, cx| {
                f.reset(cx);
                f.start_task_focus(task_id, "Write release notes", window, cx);
            });
        });
    })
    .expect("window");
    cx.run_until_parked();

    let is_running = cx.read(|cx| {
        app.shell
            .read(cx)
            .focus_ctrl()
            .read(cx)
            .timer()
            .is_running()
    });
    assert!(is_running, "focus timer should be running");

    // Switching away to Notes keeps the timer running and shows the title-bar pill.
    dispatch(&app, cx, GoNotes);
    let pill_visible = cx
        .update_window(app.window, |_, window, _| {
            window.try_find("focus-pill-open").is_some()
        })
        .expect("window");
    assert!(
        pill_visible,
        "active focus pill should appear in title bar off Focus screen"
    );

    // Pause via action, finish, and verify the pill hides when idle.
    dispatch(&app, cx, FocusToggle);
    assert!(
        cx.read(|cx| app.shell.read(cx).focus_ctrl().read(cx).timer().is_paused()),
        "FocusToggle action should pause a running timer"
    );

    cx.update_window(app.window, |_, window, cx| {
        app.shell.update(cx, |s, cx| {
            s.focus_ctrl().update(cx, |f, cx| {
                f.finish_now(true, cx).expect("finish");
            });
            s.navigate(Route::Focus, window, cx);
        });
    })
    .expect("window");
    cx.run_until_parked();

    let stats = cx.read(|cx| {
        lib.read(cx)
            .focus_stats_for_task(task_id)
            .expect("focus stats")
    });
    assert_eq!(stats.sessions, 1, "completed session should be persisted");
    let pill_hidden = cx
        .update_window(app.window, |_, window, _| {
            window.try_find("focus-pill-open").is_none()
        })
        .expect("window");
    assert!(pill_hidden, "header pill should hide when idle");
}

// ---- Ottero Cloud account ------------------------------------------------

use ottero_desktop::account::fake::FakeAccountApi;
use ottero_desktop::views::account::{AccountView, Screen};

const ACCOUNT_EMAIL: &str = "sam@example.com";
const ACCOUNT_PASSWORD: &str = "river-lantern-42";

/// The app with an in-memory account server and cheap Argon2.
fn launch_with_account(cx: &mut TestAppContext) -> (App, std::sync::Arc<FakeAccountApi>) {
    let app = launch(cx);
    let fake = std::sync::Arc::new(FakeAccountApi::new());
    attach_fake(&app, &fake, cx);
    (app, fake)
}

fn attach_fake(app: &App, fake: &std::sync::Arc<FakeAccountApi>, cx: &mut TestAppContext) {
    let api: std::sync::Arc<dyn ottero_desktop::account::client::AccountApi> = fake.clone();
    library(app, cx).update(cx, |lib, _| {
        lib.set_account_api_for_tests(api, ottero_crypto::account::AccountKdf::INSECURE_TEST);
    });
}

fn account_view(app: &App, cx: &mut TestAppContext) -> Entity<AccountView> {
    cx.read(|cx| app.shell.read(cx).settings().read(cx).account().clone())
}

fn account_screen(app: &App, cx: &mut TestAppContext) -> Screen {
    let view = account_view(app, cx);
    cx.read(|cx| view.read(cx).screen(cx))
}

fn account_error(app: &App, cx: &mut TestAppContext) -> Option<String> {
    let lib = library(app, cx);
    cx.read(|cx| lib.read(cx).account().error().map(|(_, m)| m.to_owned()))
}

fn has_label(app: &App, cx: &mut TestAppContext, label: &str) -> bool {
    use gpui_kit::test::TestQueryExt;
    cx.update_window(app.window, |_, window, cx| {
        window.render_frame(cx);
        window.try_find_by_label(label).is_some()
    })
    .expect("window")
}

fn shows_secret_key_field(app: &App, cx: &mut TestAppContext) -> bool {
    let view = account_view(app, cx);
    cx.read(|cx| view.read(cx).needs_secret_key(cx))
}

/// Create account → code (one wrong try) → Emergency Kit → signed in.
/// Returns the Secret Key shown in the kit.
fn sign_up_through_the_ui(app: &App, fake: &FakeAccountApi, cx: &mut TestAppContext) -> String {
    use ottero_desktop::actions::CreateAccount;
    dispatch(app, cx, CreateAccount);
    assert_eq!(account_screen(app, cx), Screen::SignUp);
    // Focus starts in Email; Tab walks Email → Password → Confirm.
    type_text(app, cx, ACCOUNT_EMAIL);
    press(app, cx, "tab");
    type_text(app, cx, ACCOUNT_PASSWORD);
    press(app, cx, "tab");
    type_text(app, cx, "something-else-1");
    press(app, cx, "enter");
    assert_eq!(
        account_error(app, cx).as_deref(),
        Some("The passwords don't match.")
    );
    assert_eq!(account_screen(app, cx), Screen::SignUp);

    // Fix the confirmation (select all, retype) and submit with Enter.
    press(app, cx, &secondary("a"));
    type_text(app, cx, ACCOUNT_PASSWORD);
    press(app, cx, "enter");
    assert_eq!(account_screen(app, cx), Screen::Code);
    assert!(has_label(app, cx, "Verify"));

    // A wrong code stays on the step with a message by the field.
    type_text(app, cx, "000000");
    assert_eq!(account_screen(app, cx), Screen::Code);
    assert!(
        account_error(app, cx).is_some_and(|m| m.contains("code")),
        "wrong code explained: {:?}",
        account_error(app, cx)
    );
    // The right code, pasted with a space, submits on the sixth digit.
    press(app, cx, &secondary("a"));
    let code = fake.code_for(ACCOUNT_EMAIL).expect("code sent");
    type_text(app, cx, &format!("{} {}", &code[..3], &code[3..]));
    assert_eq!(account_screen(app, cx), Screen::EmergencyKit);

    let lib = library(app, cx);
    let (secret, recovery) = cx.read(|cx| {
        let account = lib.read(cx).account();
        (
            account.secret_key_text().map(|k| k.to_string()),
            account.recovery_code_text().map(|k| k.to_string()),
        )
    });
    let secret = secret.expect("Secret Key shown");
    assert!(secret.starts_with("O1-") && secret.len() == 33);
    assert!(recovery.is_some_and(|r| r.split('-').count() == 8));
    assert!(!fake.has_keys(ACCOUNT_EMAIL), "keys wait for the kit");
    click(app, cx, "account-kit-secret");
    assert_eq!(
        cx.read(|cx| cx.read_from_clipboard().and_then(|c| c.text())),
        Some(secret.clone()),
        "Copy puts the Secret Key on the clipboard"
    );

    // Continue does nothing until the checkbox is ticked.
    click(app, cx, "account-kit-continue");
    assert_eq!(account_screen(app, cx), Screen::EmergencyKit);
    click(app, cx, "account-kit-saved");
    click(app, cx, "account-kit-continue");
    assert_eq!(account_screen(app, cx), Screen::SignedIn);
    assert!(fake.has_keys(ACCOUNT_EMAIL));
    secret
}

#[gpui_kit::test]
fn account_signed_out_page_explains_cloud_and_opens_each_form(cx: &mut TestAppContext) {
    use ottero_desktop::actions::{OpenAccountSettings, SignInToCloud};
    let (app, _fake) = launch_with_account(cx);
    dispatch(&app, cx, GoSettings);
    assert_eq!(route(&app, cx), Route::Settings);
    assert_eq!(account_screen(&app, cx), Screen::Intro, "Account is first");
    assert!(has_label(&app, cx, "Create account"));
    assert!(has_label(&app, cx, "Sign in"));

    click(&app, cx, "account-create");
    assert_eq!(account_screen(&app, cx), Screen::SignUp);
    press(&app, cx, "escape");
    assert_eq!(account_screen(&app, cx), Screen::Intro, "Escape goes back");

    dispatch(&app, cx, GoToday);
    dispatch(&app, cx, SignInToCloud);
    assert_eq!(route(&app, cx), Route::Settings);
    assert_eq!(account_screen(&app, cx), Screen::SignIn);
    // An unknown email on this device needs the Secret Key field.
    type_text(&app, cx, "new@example.com");
    assert!(shows_secret_key_field(&app, cx));
    click(&app, cx, "account-sign-in-back");
    assert_eq!(account_screen(&app, cx), Screen::Intro);

    dispatch(&app, cx, OpenAccountSettings);
    assert_eq!(account_screen(&app, cx), Screen::Intro);
}

#[gpui_kit::test]
fn account_sign_up_verifies_the_code_saves_the_kit_and_signs_out(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    let secret = sign_up_through_the_ui(&app, &fake, cx);

    // Signed in: email, plan, this device; unlocked for this run.
    assert!(has_label(&app, cx, "Sign out…"));
    let lib = library(&app, cx);
    cx.read(|cx| {
        let account = lib.read(cx).account();
        assert!(account.is_unlocked());
        assert!(account.sessions_loaded());
        assert_eq!(account.sessions().len(), 1);
        assert_eq!(account.state().email(), Some(ACCOUNT_EMAIL));
    });
    let stored = std::fs::read_to_string(app.paths.data_dir.join("account.json")).expect("file");
    assert!(stored.contains(&secret), "Secret Key kept on this device");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(app.paths.data_dir.join("account.json"))
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // Owner plan shows after a refresh.
    fake.grant_owner(ACCOUNT_EMAIL);
    lib.update(cx, |lib, cx| lib.refresh_account(cx));
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(
            lib.read(cx).account().state(),
            &ottero_core::account::AccountState::SignedIn {
                email: ACCOUNT_EMAIL.into(),
                plan: ottero_core::account::Plan::Owner,
            }
        );
    });

    // Sign out asks first, then keeps the Secret Key but drops the session.
    click(&app, cx, "account-sign-out");
    assert!(dialog_open(&app, cx), "sign out is confirmed");
    press(&app, cx, "enter");
    assert_eq!(account_screen(&app, cx), Screen::Intro);
    assert_eq!(fake.session_count(), 0, "server session ended");
    let stored = std::fs::read_to_string(app.paths.data_dir.join("account.json")).expect("file");
    assert!(stored.contains(&secret));
    assert!(!stored.contains("token"), "no session token left");

    // Signing in again on this device needs only the password.
    // The email is kept, so focus starts in Password.
    dispatch(&app, cx, ottero_desktop::actions::SignInToCloud);
    assert!(!shows_secret_key_field(&app, cx), "key is stored here");
    type_text(&app, cx, "wrong-password-1");
    press(&app, cx, "enter");
    assert_eq!(account_screen(&app, cx), Screen::SignIn);
    assert_eq!(
        account_error(&app, cx).as_deref(),
        Some("Wrong email or password.")
    );
    press(&app, cx, &secondary("a"));
    type_text(&app, cx, ACCOUNT_PASSWORD);
    press(&app, cx, "enter");
    assert_eq!(
        account_screen(&app, cx),
        Screen::SignedIn,
        "{:?}",
        account_error(&app, cx)
    );
}

#[gpui_kit::test]
fn account_second_device_signs_in_with_the_secret_key(cx: &mut TestAppContext) {
    let (first, fake) = launch_with_account(cx);
    let secret = sign_up_through_the_ui(&first, &fake, cx);
    drop(first);

    // A different computer: new data folder, same server.
    let (app, _) = launch_with_account(cx);
    attach_fake(&app, &fake, cx);
    dispatch(&app, cx, ottero_desktop::actions::SignInToCloud);
    type_text(&app, cx, ACCOUNT_EMAIL);
    press(&app, cx, "tab");
    type_text(&app, cx, ACCOUNT_PASSWORD);
    assert!(shows_secret_key_field(&app, cx), "asks for the key");

    // A typo in the key is caught before any network call.
    press(&app, cx, "tab");
    type_text(&app, cx, "O1-ABC");
    press(&app, cx, "enter");
    assert_eq!(account_screen(&app, cx), Screen::SignIn);
    assert!(account_error(&app, cx).is_some());

    // Another account's key: the server refuses the derived login.
    press(&app, cx, &secondary("a"));
    let other = ottero_crypto::account::AccountSecretKey::generate().expect("key");
    type_text(&app, cx, &other.display());
    press(&app, cx, "enter");
    assert_eq!(account_screen(&app, cx), Screen::SignIn);
    assert_eq!(
        account_error(&app, cx).as_deref(),
        Some("Wrong password or Secret Key. Check both and try again.")
    );

    // The real key, typed lower case without dashes, unwraps the account key.
    press(&app, cx, &secondary("a"));
    type_text(&app, cx, &secret.replace('-', "").to_lowercase());
    press(&app, cx, "enter");
    assert_eq!(account_screen(&app, cx), Screen::SignedIn);
    let lib = library(&app, cx);
    cx.read(|cx| {
        let account = lib.read(cx).account();
        assert!(account.is_unlocked(), "password + Secret Key proven");
        assert_eq!(
            account.secret_key_text().map(|k| k.to_string()),
            Some(secret.clone())
        );
    });
    assert_eq!(fake.session_count(), 2, "both devices listed");
}

#[gpui_kit::test]
fn account_offline_sign_up_keeps_the_form_and_explains(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    fake.set_offline(true);
    dispatch(&app, cx, ottero_desktop::actions::CreateAccount);
    type_text(&app, cx, ACCOUNT_EMAIL);
    press(&app, cx, "tab");
    type_text(&app, cx, ACCOUNT_PASSWORD);
    press(&app, cx, "tab");
    type_text(&app, cx, ACCOUNT_PASSWORD);
    press(&app, cx, "enter");
    assert_eq!(account_screen(&app, cx), Screen::SignUp);
    let message = account_error(&app, cx).expect("offline message");
    assert!(
        message.contains("connection") || message.contains("offline"),
        "{message}"
    );
    fake.set_offline(false);
    // Typing clears the error; Enter retries with the same input.
    press(&app, cx, "enter");
    assert_eq!(account_screen(&app, cx), Screen::Code);
}

#[gpui_kit::test]
fn account_after_restart_is_signed_in_but_locked_until_the_password(cx: &mut TestAppContext) {
    let (first, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&first, &fake, cx);
    let App {
        window: _,
        shell: _,
        _dir: dir,
        paths,
    } = first;

    // Same data folder, new process: the token and Secret Key are on disk,
    // the account key is not.
    let api: std::sync::Arc<dyn ottero_desktop::account::client::AccountApi> = fake.clone();
    let (window, shell) = cx.update(|cx| {
        ottero_desktop::start_with_account_api(
            &paths,
            Store::open_in_memory().expect("store"),
            api,
            cx,
        )
        .expect("start")
    });
    let app = App {
        window,
        shell,
        _dir: dir,
        paths,
    };
    attach_fake(&app, &fake, cx);
    dispatch(&app, cx, ottero_desktop::actions::OpenAccountSettings);
    assert_eq!(account_screen(&app, cx), Screen::SignedIn);
    let lib = library(&app, cx);
    assert!(!cx.read(|cx| lib.read(cx).account().is_unlocked()));
    assert!(
        cx.read(|cx| lib.read(cx).account().secret_key_text().is_none()),
        "Secret Key hidden until the password is entered"
    );
    assert!(has_label(&app, cx, "Unlock"));

    let wrong = lib.update(cx, |lib, cx| {
        lib.unlock_account(
            ottero_crypto::SecretString::from("nope-nope-nope".to_owned()),
            cx,
        )
    });
    assert!(finish(wrong, cx).is_err());
    let right = lib.update(cx, |lib, cx| {
        lib.unlock_account(
            ottero_crypto::SecretString::from(ACCOUNT_PASSWORD.to_owned()),
            cx,
        )
    });
    assert_eq!(finish(right, cx), Ok(()));
    assert!(cx.read(|cx| lib.read(cx).account().secret_key_text().is_some()));
}

fn finish<T: 'static>(task: gpui_kit::Task<T>, cx: &mut TestAppContext) -> T {
    let out = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = out.clone();
    cx.update(|cx| {
        cx.spawn(async move |_| *slot.borrow_mut() = Some(task.await))
            .detach();
    });
    cx.run_until_parked();
    out.borrow_mut().take().expect("task finished")
}

#[gpui_kit::test]
fn account_other_device_can_be_signed_out_after_confirming(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    fake.add_session(ACCOUNT_EMAIL, "Ottero/0.1.3 (Windows)");
    let lib = library(&app, cx);
    lib.update(cx, |lib, cx| lib.refresh_account(cx));
    cx.run_until_parked();
    let other = cx.read(|cx| {
        let account = lib.read(cx).account();
        assert_eq!(account.sessions().len(), 2);
        let current = account.current_token().expect("token");
        account
            .sessions()
            .iter()
            .find(|s| !s.is_current(&current))
            .map(|s| s.id.clone())
            .expect("other device")
    });
    let id: &'static str = leak(format!("session-sign-out-{other}"));
    // Devices is the last group: scroll the page to it, as a user would.
    cx.update_window(app.window, |_, window, cx| {
        window.render_frame(cx);
        window.scroll(
            "account-sign-out",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-600.))),
            cx,
        );
    })
    .expect("window");
    cx.run_until_parked();
    click(&app, cx, id);
    assert!(dialog_open(&app, cx), "revoking asks first");
    press(&app, cx, "enter");
    cx.run_until_parked();
    assert_eq!(fake.session_count(), 1, "only this device left");
    assert_eq!(cx.read(|cx| lib.read(cx).account().sessions().len()), 1);
    assert_eq!(account_screen(&app, cx), Screen::SignedIn);
}

/// Account B signed in on this device (locked), standing in for "A signed
/// out and B signed in" while one of A's requests is still in flight.
fn other_account_signed_in() -> ottero_desktop::account::AccountSession {
    use ottero_desktop::account::store::{AccountFile, PlanRecord, SignedIn};
    let mut file = AccountFile::in_memory();
    file.set_signed_in(SignedIn {
        email: "bea@example.com".into(),
        token: "token-of-bea".into(),
        salt: "AAAAAAAAAAAAAAAAAAAAAA".into(),
        wrapped_by_password: None,
        plan: Some(PlanRecord {
            role: "user".into(),
            cloud_source: None,
            cloud_ends_at: None,
        }),
        two_factor: Some(false),
    })
    .expect("signed in");
    ottero_desktop::account::AccountSession::new(file, std::sync::Arc::new(FakeAccountApi::new()))
}

#[gpui_kit::test]
fn account_late_refresh_for_a_previous_account_is_dropped(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    fake.grant_owner(ACCOUNT_EMAIL);
    let lib = library(&app, cx);
    // A's refresh starts (it would report "Owner"), then B takes over before it lands.
    lib.update(cx, |lib, cx| {
        lib.refresh_account(cx);
        lib.set_account_session(other_account_signed_in());
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let account = lib.read(cx).account();
        assert_eq!(
            account.state().email(),
            Some("bea@example.com"),
            "B stays signed in"
        );
        assert!(
            matches!(
                account.state(),
                ottero_core::account::AccountState::SignedIn {
                    plan: ottero_core::account::Plan::Free,
                    ..
                }
            ),
            "A's plan isn't copied into B"
        );
        assert!(
            account.sessions().is_empty(),
            "A's devices aren't shown as B's"
        );
        assert!(account.notice().is_none());
    });
}

#[gpui_kit::test]
fn account_late_unlock_for_a_previous_account_is_dropped(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    let lib = library(&app, cx);
    lib.update(cx, |lib, cx| lib.lock_account(cx));
    // A's unlock is still deriving when B signs in on this device.
    let unlock = lib.update(cx, |lib, cx| {
        let task = lib.unlock_account(
            ottero_crypto::SecretString::from(ACCOUNT_PASSWORD.to_owned()),
            cx,
        );
        lib.set_account_session(other_account_signed_in());
        task
    });
    assert!(
        finish(unlock, cx).is_err(),
        "the late unlock reports that the account changed"
    );
    cx.read(|cx| {
        let account = lib.read(cx).account();
        assert_eq!(account.state().email(), Some("bea@example.com"));
        assert!(
            !account.is_unlocked(),
            "A's account key never enters B's session"
        );
    });
}

#[gpui_kit::test]
fn account_late_unlock_after_locking_again_stays_locked(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    let lib = library(&app, cx);
    lib.update(cx, |lib, cx| lib.lock_account(cx));
    let unlock = lib.update(cx, |lib, cx| {
        let task = lib.unlock_account(
            ottero_crypto::SecretString::from(ACCOUNT_PASSWORD.to_owned()),
            cx,
        );
        lib.account_sign_out(cx).expect("signed out");
        task
    });
    assert!(finish(unlock, cx).is_err());
    cx.read(|cx| {
        let account = lib.read(cx).account();
        assert!(!account.is_unlocked());
        assert!(!account.state().is_signed_in());
    });
}

// ---- Two-step verification ----------------------------------------------

use ottero_desktop::views::account::{TwoStepDialog, TwoStepStep};

fn two_step_dialog(app: &App, cx: &mut TestAppContext) -> Option<Entity<TwoStepDialog>> {
    let view = account_view(app, cx);
    cx.read(|cx| view.read(cx).two_step_dialog())
}

fn dialog_step(app: &App, cx: &mut TestAppContext) -> Option<TwoStepStep> {
    let dialog = two_step_dialog(app, cx)?;
    Some(cx.read(|cx| dialog.read(cx).step()))
}

/// The code an authenticator app shows now for an `otpauth://` link.
fn authenticator_code(uri: &str) -> String {
    let now = ottero_core::clock::now().as_second().cast_unsigned();
    ottero_core::vault::totp::parse(uri)
        .expect("otpauth link")
        .code_at(now)
        .code
}

fn current_token(app: &App, cx: &mut TestAppContext) -> String {
    let lib = library(app, cx);
    cx.read(|cx| {
        lib.read(cx)
            .account()
            .current_token()
            .map(|t| t.expose().to_owned())
            .expect("token")
    })
}

fn password(text: &str) -> ottero_crypto::SecretString {
    ottero_crypto::SecretString::from(text.to_owned())
}

/// Turns two-step on through the `Library` (the UI path has its own test).
/// Returns the setup link and the backup codes.
fn turn_on_two_step(app: &App, cx: &mut TestAppContext) -> (String, Vec<String>) {
    let lib = library(app, cx);
    let start = lib.update(cx, |lib, cx| {
        lib.start_two_step_setup(password(ACCOUNT_PASSWORD), cx)
    });
    let setup = finish(start, cx).expect("setup");
    let uri = setup.uri().to_owned();
    let codes = setup.backup_codes().iter().map(|c| c.to_string()).collect();
    let code = authenticator_code(&uri);
    let confirm = lib.update(cx, |lib, cx| lib.confirm_two_step_setup(&code, cx));
    finish(confirm, cx).expect("on");
    (uri, codes)
}

fn sign_in_with_password(app: &App, cx: &mut TestAppContext) {
    dispatch(app, cx, ottero_desktop::actions::SignInToCloud);
    assert_eq!(account_screen(app, cx), Screen::SignIn);
    // The email is kept and this device has the Secret Key: only the password.
    assert!(!shows_secret_key_field(app, cx));
    type_text(app, cx, ACCOUNT_PASSWORD);
    press(app, cx, "enter");
}

fn sign_out(app: &App, cx: &mut TestAppContext) {
    let lib = library(app, cx);
    lib.update(cx, |lib, cx| lib.account_sign_out(cx))
        .expect("signed out");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn account_two_step_turns_on_with_a_qr_code_and_gates_on_saved_backup_codes(
    cx: &mut TestAppContext,
) {
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    let lib = library(&app, cx);
    assert_eq!(
        cx.read(|cx| lib.read(cx).account().two_step_on()),
        Some(false)
    );
    assert!(has_label(&app, cx, "Turn on…"));

    click(&app, cx, "account-two-step-on");
    assert!(dialog_open(&app, cx));
    assert_eq!(dialog_step(&app, cx), Some(TwoStepStep::Password));
    // A wrong password is caught on this device, before any request.
    type_text(&app, cx, "not-my-password-9");
    press(&app, cx, "enter");
    assert_eq!(dialog_step(&app, cx), Some(TwoStepStep::Password));
    let dialog = two_step_dialog(&app, cx).expect("dialog");
    assert_eq!(
        cx.read(|cx| dialog.read(cx).error().map(str::to_owned))
            .as_deref(),
        Some("That password isn't right.")
    );

    press(&app, cx, &secondary("a"));
    type_text(&app, cx, ACCOUNT_PASSWORD);
    press(&app, cx, "enter");
    assert_eq!(dialog_step(&app, cx), Some(TwoStepStep::Scan));
    let uri = cx
        .read(|cx| dialog.read(cx).setup_uri().map(str::to_owned))
        .expect("link");
    assert!(uri.starts_with("otpauth://totp/Ottero:"), "{uri}");
    let shown = cx
        .read(|cx| dialog.read(cx).qr().cloned())
        .expect("QR code");
    assert_eq!(
        Some(shown),
        ottero_desktop::views::qr::QrMatrix::encode(&uri),
        "the QR code holds the setup link"
    );
    let key_shown = cx
        .update_window(app.window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find("two-step-copy-key").is_some()
        })
        .expect("window");
    assert!(key_shown, "setup key for manual entry");
    assert!(!fake.two_factor_on(ACCOUNT_EMAIL), "not on until confirmed");

    // Escape still works here: nothing is on yet.
    let old_token = current_token(&app, cx);
    // A wrong code (six digits submit by themselves) stays on the step.
    type_text(&app, cx, "000000");
    assert_eq!(dialog_step(&app, cx), Some(TwoStepStep::Scan));
    assert!(
        cx.read(|cx| dialog.read(cx).error().map(str::to_owned))
            .is_some_and(|e| e.contains("isn't right")),
        "wrong code explained"
    );
    press(&app, cx, &secondary("a"));
    type_text(&app, cx, &authenticator_code(&uri));
    assert_eq!(dialog_step(&app, cx), Some(TwoStepStep::BackupCodes));
    assert!(fake.two_factor_on(ACCOUNT_EMAIL));
    let codes = cx.read(|cx| dialog.read(cx).backup_codes());
    assert_eq!(codes.len(), 10);
    assert_eq!(codes, fake.backup_codes(ACCOUNT_EMAIL));

    // The server replaced the session; the new token is the one kept.
    let new_token = current_token(&app, cx);
    assert_ne!(new_token, old_token);
    let api: &dyn ottero_desktop::account::client::AccountApi = &*fake;
    let session = |t: &str| ottero_desktop::account::client::SessionToken::new(t.to_owned());
    assert!(fake.is_session(&session(&new_token)));
    assert!(!fake.is_session(&session(&old_token)), "old token ended");
    assert!(api.me(&session(&new_token)).is_ok_and(|me| me.two_factor));
    let stored = std::fs::read_to_string(app.paths.data_dir.join("account.json")).expect("file");
    assert!(stored.contains(&new_token) && !stored.contains(&old_token));
    assert!(!stored.contains(&codes[0]), "backup codes are never stored");

    // Copy all, then the dialog only closes through Done after the checkbox.
    click(&app, cx, "two-step-copy-codes");
    assert_eq!(
        cx.read(|cx| cx.read_from_clipboard().and_then(|c| c.text())),
        Some(codes.join("\n"))
    );
    press(&app, cx, "escape");
    assert!(dialog_open(&app, cx), "Escape can't lose the codes");
    press(&app, cx, "enter");
    assert!(
        dialog_open(&app, cx),
        "Enter isn't Done before the checkbox"
    );
    click(&app, cx, "two-step-done");
    assert!(dialog_open(&app, cx), "Done waits for the checkbox");
    click(&app, cx, "two-step-codes-saved");
    click(&app, cx, "two-step-done");
    assert!(!dialog_open(&app, cx), "Done closes once saved");
    assert_eq!(
        cx.read(|cx| lib.read(cx).account().two_step_on()),
        Some(true)
    );
    assert!(has_label(&app, cx, "Turn off…"));
    assert!(has_label(&app, cx, "New backup codes…"));
}

#[gpui_kit::test]
fn account_two_step_sign_in_asks_for_the_code_and_backup_codes_work_once(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    let (_, codes) = turn_on_two_step(&app, cx);
    sign_out(&app, cx);
    let sessions_before = fake.session_count();

    sign_in_with_password(&app, cx);
    assert_eq!(account_screen(&app, cx), Screen::TwoStep);
    assert!(has_label(&app, cx, "Use a backup code"));
    assert_eq!(
        fake.session_count(),
        sessions_before,
        "no session from the password alone"
    );
    // Wrong code: the message sits under the field, the step stays.
    type_text(&app, cx, "000000");
    assert_eq!(account_screen(&app, cx), Screen::TwoStep);
    assert_eq!(
        account_error(&app, cx).as_deref(),
        Some("That code isn't right. Check your authenticator app and try again.")
    );
    press(&app, cx, &secondary("a"));
    let code = fake.authenticator_code(ACCOUNT_EMAIL).expect("code");
    type_text(&app, cx, &format!("{} {}", &code[..3], &code[3..]));
    assert_eq!(
        account_screen(&app, cx),
        Screen::SignedIn,
        "{:?}",
        account_error(&app, cx)
    );
    let lib = library(&app, cx);
    assert!(cx.read(|cx| lib.read(cx).account().is_unlocked()));

    // A backup code, typed without its dash, signs in once.
    sign_out(&app, cx);
    sign_in_with_password(&app, cx);
    assert_eq!(account_screen(&app, cx), Screen::TwoStep);
    click(&app, cx, "account-two-step-switch");
    let view = account_view(&app, cx);
    assert!(cx.read(|cx| view.read(cx).uses_backup_code()));
    type_text(&app, cx, &codes[0].replace('-', ""));
    press(&app, cx, "enter");
    assert_eq!(
        account_screen(&app, cx),
        Screen::SignedIn,
        "{:?}",
        account_error(&app, cx)
    );
    assert_eq!(fake.backup_codes(ACCOUNT_EMAIL).len(), 9);

    sign_out(&app, cx);
    sign_in_with_password(&app, cx);
    click(&app, cx, "account-two-step-switch");
    type_text(&app, cx, &codes[0]);
    press(&app, cx, "enter");
    assert_eq!(account_screen(&app, cx), Screen::TwoStep);
    assert_eq!(
        account_error(&app, cx).as_deref(),
        Some("That backup code isn't right, or it was already used.")
    );
    // Back returns to the password with the email kept.
    press(&app, cx, "escape");
    assert_eq!(account_screen(&app, cx), Screen::SignIn);
    assert!(account_error(&app, cx).is_none());
    let email = cx.read(|cx| view.read(cx).screen(cx));
    assert_eq!(email, Screen::SignIn);

    // Five wrong codes lock two-step verification for a while.
    type_text(&app, cx, ACCOUNT_PASSWORD);
    press(&app, cx, "enter");
    assert_eq!(account_screen(&app, cx), Screen::TwoStep);
    for _ in 0..4 {
        press(&app, cx, &secondary("a"));
        type_text(&app, cx, "000000");
    }
    press(&app, cx, &secondary("a"));
    let code = fake.authenticator_code(ACCOUNT_EMAIL).expect("code");
    type_text(&app, cx, &code);
    assert_eq!(account_screen(&app, cx), Screen::TwoStep);
    assert_eq!(
        account_error(&app, cx).as_deref(),
        Some("Too many wrong codes. Try again in 15 minutes."),
        "locked even for the right code"
    );
}

/// The steps a returning user sees: after a restart only the password
/// (no Secret Key, no authenticator code); a new sign-in on this device
/// needs the password and the code, never the Secret Key again.
#[gpui_kit::test]
fn account_two_step_is_asked_when_signing_in_not_when_unlocking(cx: &mut TestAppContext) {
    let (first, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&first, &fake, cx);
    turn_on_two_step(&first, cx);
    let App {
        window: _,
        shell: _,
        _dir: dir,
        paths,
    } = first;
    let api: std::sync::Arc<dyn ottero_desktop::account::client::AccountApi> = fake.clone();
    let (window, shell) = cx.update(|cx| {
        ottero_desktop::start_with_account_api(
            &paths,
            Store::open_in_memory().expect("store"),
            api,
            cx,
        )
        .expect("start")
    });
    let app = App {
        window,
        shell,
        _dir: dir,
        paths,
    };
    attach_fake(&app, &fake, cx);
    dispatch(&app, cx, ottero_desktop::actions::OpenAccountSettings);
    assert_eq!(
        account_screen(&app, cx),
        Screen::SignedIn,
        "still signed in"
    );
    let lib = library(&app, cx);
    assert_eq!(
        cx.read(|cx| lib.read(cx).account().two_step_on()),
        Some(true)
    );
    let unlock = lib.update(cx, |lib, cx| {
        lib.unlock_account(password(ACCOUNT_PASSWORD), cx)
    });
    assert_eq!(finish(unlock, cx), Ok(()), "the password alone unlocks");
    assert_eq!(account_screen(&app, cx), Screen::SignedIn);

    // Signed out: email and password (the Secret Key is remembered), then the code.
    sign_out(&app, cx);
    dispatch(&app, cx, ottero_desktop::actions::SignInToCloud);
    type_text(&app, cx, ACCOUNT_EMAIL);
    assert!(
        !shows_secret_key_field(&app, cx),
        "Secret Key kept on this device"
    );
    press(&app, cx, "tab");
    type_text(&app, cx, ACCOUNT_PASSWORD);
    press(&app, cx, "enter");
    assert_eq!(account_screen(&app, cx), Screen::TwoStep);
    type_text(
        &app,
        cx,
        &fake.authenticator_code(ACCOUNT_EMAIL).expect("code"),
    );
    assert_eq!(account_screen(&app, cx), Screen::SignedIn);
}

#[gpui_kit::test]
fn account_two_step_new_backup_codes_and_turning_off(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    let (_, first_codes) = turn_on_two_step(&app, cx);
    let lib = library(&app, cx);

    // New backup codes: password, then the new set (the old ones stop working).
    dispatch(&app, cx, ottero_desktop::actions::NewBackupCodes);
    assert_eq!(dialog_step(&app, cx), Some(TwoStepStep::Password));
    type_text(&app, cx, ACCOUNT_PASSWORD);
    press(&app, cx, "enter");
    assert_eq!(dialog_step(&app, cx), Some(TwoStepStep::BackupCodes));
    let dialog = two_step_dialog(&app, cx).expect("dialog");
    let codes = cx.read(|cx| dialog.read(cx).backup_codes());
    assert_eq!(codes, fake.backup_codes(ACCOUNT_EMAIL));
    assert!(!codes.contains(&first_codes[0]));
    click(&app, cx, "two-step-codes-saved");
    click(&app, cx, "two-step-done");
    assert!(!dialog_open(&app, cx));

    // Turn off: confirmed first, then the password; the session is replaced.
    let old_token = current_token(&app, cx);
    click(&app, cx, "account-two-step-off");
    assert!(dialog_open(&app, cx), "turning off is confirmed");
    assert!(
        two_step_dialog(&app, cx).is_none(),
        "confirmation comes first"
    );
    press(&app, cx, "enter");
    assert_eq!(dialog_step(&app, cx), Some(TwoStepStep::Password));
    assert!(has_label(&app, cx, "Turn off"));
    type_text(&app, cx, ACCOUNT_PASSWORD);
    press(&app, cx, "enter");
    assert!(!dialog_open(&app, cx));
    assert!(!fake.two_factor_on(ACCOUNT_EMAIL));
    assert_eq!(
        cx.read(|cx| lib.read(cx).account().two_step_on()),
        Some(false)
    );
    let new_token = current_token(&app, cx);
    assert_ne!(new_token, old_token);
    assert!(
        fake.is_session(&ottero_desktop::account::client::SessionToken::new(
            new_token
        ))
    );
    assert!(has_label(&app, cx, "Turn on…"));

    // Signing in again needs only the password.
    sign_out(&app, cx);
    sign_in_with_password(&app, cx);
    assert_eq!(account_screen(&app, cx), Screen::SignedIn);
}

#[gpui_kit::test]
fn account_late_two_step_confirm_for_a_previous_account_is_dropped(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    let lib = library(&app, cx);
    let start = lib.update(cx, |lib, cx| {
        lib.start_two_step_setup(password(ACCOUNT_PASSWORD), cx)
    });
    let uri = finish(start, cx).expect("setup").uri().to_owned();
    // A's confirmation is in flight when B signs in on this device.
    let confirm = lib.update(cx, |lib, cx| {
        let task = lib.confirm_two_step_setup(&authenticator_code(&uri), cx);
        lib.set_account_session(other_account_signed_in());
        task
    });
    assert!(finish(confirm, cx).is_err(), "reported, not applied");
    cx.run_until_parked();
    cx.read(|cx| {
        let account = lib.read(cx).account();
        assert_eq!(account.state().email(), Some("bea@example.com"));
        assert_eq!(
            account
                .current_token()
                .map(|t| t.expose().to_owned())
                .as_deref(),
            Some("token-of-bea"),
            "A's replacement token never lands in B's sign-in"
        );
        assert_eq!(account.two_step_on(), Some(false));
    });
    assert_eq!(
        fake.session_count(),
        0,
        "A's orphaned new session was signed out"
    );

    // The same for a late start: no setup reaches B.
    let start = lib.update(cx, |lib, cx| {
        lib.set_account_session(other_account_signed_in());
        lib.start_two_step_setup(password(ACCOUNT_PASSWORD), cx)
    });
    assert!(finish(start, cx).is_err());
}

#[gpui_kit::test]
fn account_late_two_step_code_after_going_back_is_dropped(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    turn_on_two_step(&app, cx);
    sign_out(&app, cx);
    sign_in_with_password(&app, cx);
    assert_eq!(account_screen(&app, cx), Screen::TwoStep);
    let code = fake.authenticator_code(ACCOUNT_EMAIL).expect("code");
    let lib = library(&app, cx);
    // The right code is checking when the user goes back.
    lib.update(cx, |lib, cx| {
        lib.verify_two_step_code(
            &code,
            ottero_desktop::account::client::SecondFactor::Authenticator,
            cx,
        );
        lib.cancel_account_flow(cx);
    });
    cx.run_until_parked();
    assert_ne!(account_screen(&app, cx), Screen::SignedIn);
    assert!(cx.read(|cx| lib.read(cx).account().current_token().is_none()));
    assert_eq!(fake.session_count(), 0, "the session it created was ended");
}

// ---- Passwords under the Ottero account (ADR 0014) ------------------------

use ottero_core::vault::lock::{LockState, Opener};
use ottero_desktop::views::passwords::Screen as VaultScreen;

const MASTER: &str = "plum-orbit-candle-9-meadow";

/// The app on a database file in `dir` (so a "restart" keeps the vault),
/// with the fake account server and cheap Argon2 for both keys.
fn launch_on_disk(
    cx: &mut TestAppContext,
    dir: tempfile::TempDir,
    fake: &std::sync::Arc<FakeAccountApi>,
) -> App {
    let paths = AppPaths::in_dir(dir.path().to_path_buf());
    std::fs::create_dir_all(dir.path().join("updates")).expect("update prefs directory");
    std::fs::write(
        dir.path().join("updates/prefs.json"),
        br#"{"automatic":false}"#,
    )
    .expect("offline test prefs");
    let store = Store::open(&paths.database).expect("store");
    let api: std::sync::Arc<dyn ottero_desktop::account::client::AccountApi> = fake.clone();
    let (window, shell) = cx.update(|cx| {
        ottero_desktop::start_with_account_api(&paths, store, api, cx).expect("start")
    });
    cx.run_until_parked();
    let app = App {
        window,
        shell,
        _dir: dir,
        paths,
    };
    attach_fake(&app, fake, cx);
    library(&app, cx).update(cx, |l, _| {
        l.set_kdf_params_for_tests(ottero_crypto::KdfParams::MINIMUM)
    });
    app
}

/// Closes the window and starts again on the same data folder.
fn restart(app: App, fake: &std::sync::Arc<FakeAccountApi>, cx: &mut TestAppContext) -> App {
    let App { window, _dir, .. } = app;
    cx.update_window(window, |_, window, _| window.remove_window())
        .expect("close");
    cx.run_until_parked();
    launch_on_disk(cx, _dir, fake)
}

fn vault_screen(app: &App, cx: &mut TestAppContext) -> VaultScreen {
    let passwords = cx.read(|cx| app.shell.read(cx).passwords().clone());
    cx.read(|cx| passwords.read(cx).screen(cx))
}

fn vault_state(app: &App, cx: &mut TestAppContext) -> LockState {
    let lib = library(app, cx);
    cx.read(|cx| lib.read(cx).vault_state())
}

fn opener(app: &App, cx: &mut TestAppContext) -> Option<Opener> {
    let lib = library(app, cx);
    cx.read(|cx| lib.read(cx).vault_opener())
}

fn account_open(app: &App, cx: &mut TestAppContext) -> bool {
    let lib = library(app, cx);
    cx.read(|cx| lib.read(cx).account().is_unlocked())
}

/// A master-password vault with two logins, locked. Returns the recovery key.
fn local_vault_with_items(app: &App, cx: &mut TestAppContext) -> String {
    use ottero_core::vault::{ItemFields, ItemKind};
    let lib = library(app, cx);
    lib.update(cx, |l, _| {
        l.set_kdf_params_for_tests(ottero_crypto::KdfParams::MINIMUM)
    });
    let setup = lib.update(cx, |l, cx| l.set_up_vault(password(MASTER), cx));
    let recovery = finish(setup, cx).expect("setup").to_string();
    lib.update(cx, |l, cx| {
        for (title, pw) in [("GitHub", "gh-pass-1"), ("Bank", "bank-pass-2")] {
            let fields = ItemFields {
                title: title.into(),
                password: pw.into(),
                ..ItemFields::new(ItemKind::Login)
            };
            l.create_vault_item(&fields, cx).expect("item");
        }
    });
    lib.update(cx, |l, cx| l.lock_vault(cx));
    cx.run_until_parked();
    recovery
}

fn vault_titles(app: &App, cx: &mut TestAppContext) -> Vec<String> {
    let lib = library(app, cx);
    cx.read(|cx| {
        let mut t: Vec<String> = lib
            .read(cx)
            .vault()
            .items()
            .iter()
            .map(|i| i.title().to_owned())
            .collect();
        t.sort();
        t
    })
}

/// Signs up `email` through the `Library` (the form has its own test).
fn sign_up_as(app: &App, fake: &FakeAccountApi, email: &str, cx: &mut TestAppContext) {
    let lib = library(app, cx);
    lib.update(cx, |l, cx| {
        l.account_sign_up(email, password(ACCOUNT_PASSWORD), cx)
    });
    cx.run_until_parked();
    let code = fake.code_for(email).expect("code sent");
    lib.update(cx, |l, cx| l.verify_account_code(&code, cx));
    cx.run_until_parked();
    lib.update(cx, |l, cx| l.finish_emergency_kit(cx));
    cx.run_until_parked();
    assert!(account_open(app, cx), "{email} signed in and unlocked");
}

/// Sets a new vault up under the signed-in account from the setup screen.
/// Returns the recovery key it showed.
fn set_up_under_account(app: &App, cx: &mut TestAppContext) -> String {
    use ottero_desktop::actions::GoPasswords;
    dispatch(app, cx, GoPasswords);
    assert_eq!(vault_screen(app, cx), VaultScreen::SetupWithAccount);
    assert!(
        !element_present(app, cx, "vault-master"),
        "no master password is asked for"
    );
    click(app, cx, "vault-create-account");
    assert_eq!(vault_state(app, cx), LockState::Unlocked);
    assert_eq!(vault_screen(app, cx), VaultScreen::Recovery);
    let passwords = cx.read(|cx| app.shell.read(cx).passwords().clone());
    let key = cx
        .read(|cx| passwords.read(cx).recovery_key().map(str::to_owned))
        .expect("recovery key shown once");
    click(app, cx, "recovery-saved");
    click(app, cx, "recovery-done");
    assert_eq!(vault_screen(app, cx), VaultScreen::Unlocked);
    key
}

fn add_login(app: &App, title: &str, cx: &mut TestAppContext) {
    use ottero_core::vault::{ItemFields, ItemKind};
    let lib = library(app, cx);
    lib.update(cx, |l, cx| {
        let fields = ItemFields {
            title: title.into(),
            password: "pw-123".into(),
            ..ItemFields::new(ItemKind::Login)
        };
        l.create_vault_item(&fields, cx).expect("item");
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn vault_local_only_keeps_its_master_password(cx: &mut TestAppContext) {
    use ottero_desktop::actions::GoPasswords;
    let app = launch(cx);
    dispatch(&app, cx, GoPasswords);
    assert_eq!(vault_screen(&app, cx), VaultScreen::Setup, "not signed in");
    local_vault_with_items(&app, cx);
    assert_eq!(opener(&app, cx), Some(Opener::MasterPassword));
    dispatch(&app, cx, GoPasswords);
    assert_eq!(vault_screen(&app, cx), VaultScreen::Locked);
    assert!(element_present(&app, cx, "vault-unlock"));
    type_text(&app, cx, MASTER);
    press(&app, cx, "enter");
    assert_eq!(vault_state(&app, cx), LockState::Unlocked);
    assert_eq!(opener(&app, cx), Some(Opener::MasterPassword));
}

#[gpui_kit::test]
fn vault_links_with_the_master_password_once_wrong_then_right(cx: &mut TestAppContext) {
    use ottero_desktop::actions::GoPasswords;
    let (app, fake) = launch_with_account(cx);
    local_vault_with_items(&app, cx);
    sign_up_through_the_ui(&app, &fake, cx);
    assert_eq!(
        vault_state(&app, cx),
        LockState::Locked,
        "a local vault isn't opened by signing in"
    );

    dispatch(&app, cx, GoPasswords);
    assert_eq!(vault_screen(&app, cx), VaultScreen::Link);
    assert!(element_present(&app, cx, "vault-link-master"));
    assert!(has_label(&app, cx, "Unlock and link"));
    type_text(&app, cx, "not my master password");
    press(&app, cx, "enter");
    assert_eq!(vault_state(&app, cx), LockState::Locked);
    assert_eq!(
        opener(&app, cx),
        Some(Opener::MasterPassword),
        "nothing changed"
    );
    let passwords = cx.read(|cx| app.shell.read(cx).passwords().clone());
    assert_eq!(
        cx.read(|cx| passwords.read(cx).error().map(str::to_owned))
            .as_deref(),
        Some("That password didn't unlock the vault.")
    );

    // The wrong text is selected, so typing replaces it.
    type_text(&app, cx, MASTER);
    press(&app, cx, "enter");
    assert_eq!(vault_state(&app, cx), LockState::Unlocked);
    assert_eq!(opener(&app, cx), Some(Opener::Account));
    let lib = library(&app, cx);
    assert_eq!(
        cx.read(|cx| lib.read(cx).vault_linked_email().map(str::to_owned)),
        Some(ACCOUNT_EMAIL.to_owned())
    );
    assert_eq!(vault_titles(&app, cx), ["Bank", "GitHub"]);
    assert_eq!(
        vault_screen(&app, cx),
        VaultScreen::Unlocked,
        "it had a recovery key already"
    );

    // The master password is retired: it no longer unlocks, even after a lock.
    dispatch(&app, cx, ottero_desktop::actions::LockVault);
    let old = lib.update(cx, |l, cx| l.unlock_vault(password(MASTER), cx));
    assert!(
        finish(old, cx).is_err(),
        "there is no master password to try"
    );
    assert!(cx.read(|cx| lib.read(cx).vault_has_recovery()));
}

#[gpui_kit::test]
fn vault_link_dialog_works_while_unlocked_and_from_the_palette(cx: &mut TestAppContext) {
    use ottero_desktop::actions::{GoPasswords, UseAccountForPasswords};
    let (app, fake) = launch_with_account(cx);
    local_vault_with_items(&app, cx);
    let lib = library(&app, cx);
    let unlock = lib.update(cx, |l, cx| l.unlock_vault(password(MASTER), cx));
    assert_eq!(finish(unlock, cx).ok(), Some(true));
    sign_up_through_the_ui(&app, &fake, cx);
    assert_eq!(vault_state(&app, cx), LockState::Unlocked);
    dispatch(&app, cx, GoPasswords);
    dispatch(&app, cx, UseAccountForPasswords);
    assert!(dialog_open(&app, cx));
    type_text(&app, cx, "wrong-wrong-wrong");
    assert!(dialog_open(&app, cx), "typing keeps the dialog");
    press(&app, cx, "enter");
    assert!(dialog_open(&app, cx), "a wrong password keeps the dialog");
    assert_eq!(opener(&app, cx), Some(Opener::MasterPassword));
    press(&app, cx, &secondary("a"));
    type_text(&app, cx, MASTER);
    press(&app, cx, "enter");
    assert!(!dialog_open(&app, cx));
    assert_eq!(opener(&app, cx), Some(Opener::Account));
    assert_eq!(vault_state(&app, cx), LockState::Unlocked);
}

#[gpui_kit::test]
fn vault_new_vault_under_the_account_restarts_into_one_unlock(cx: &mut TestAppContext) {
    use ottero_desktop::actions::GoPasswords;
    let fake = std::sync::Arc::new(FakeAccountApi::new());
    let app = launch_on_disk(cx, tempfile::tempdir().expect("dir"), &fake);
    sign_up_through_the_ui(&app, &fake, cx);
    set_up_under_account(&app, cx);
    assert_eq!(opener(&app, cx), Some(Opener::Account));
    add_login(&app, "Mail", cx);

    // A new process: the account key and the vault key are both gone.
    let app = restart(app, &fake, cx);
    assert!(!account_open(&app, cx));
    assert_eq!(vault_state(&app, cx), LockState::Locked);
    dispatch(&app, cx, GoPasswords);
    assert_eq!(vault_screen(&app, cx), VaultScreen::UnlockOttero);
    assert!(element_present(&app, cx, "vault-account-unlock"));
    assert!(
        !element_present(&app, cx, "vault-unlock"),
        "no master password screen"
    );

    type_text(&app, cx, "not-the-password-1");
    press(&app, cx, "enter");
    assert!(!account_open(&app, cx));
    assert_eq!(vault_state(&app, cx), LockState::Locked);

    press(&app, cx, &secondary("a"));
    type_text(&app, cx, ACCOUNT_PASSWORD);
    press(&app, cx, "enter");
    assert!(account_open(&app, cx), "one password opens the account…");
    assert_eq!(vault_state(&app, cx), LockState::Unlocked, "…and Passwords");
    assert_eq!(vault_titles(&app, cx), ["Mail"]);
}

#[gpui_kit::test]
fn vault_linked_auto_lock_locks_the_account_too(cx: &mut TestAppContext) {
    use ottero_core::vault::lock::AutoLock;
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    set_up_under_account(&app, cx);
    let lib = library(&app, cx);
    cx.update(|cx| ottero_desktop::vault_prefs::update(cx, |p| p.auto_lock = AutoLock::OneMinute));
    lib.update(cx, |l, _| {
        l.backdate_vault_activity_for_tests(Duration::from_secs(120))
    });
    cx.executor().advance_clock(Duration::from_secs(11));
    cx.run_until_parked();
    assert_eq!(vault_state(&app, cx), LockState::Locked);
    assert!(!account_open(&app, cx), "the account key is locked with it");
    assert_eq!(vault_screen(&app, cx), VaultScreen::UnlockOttero);
    // Unlocking the account anywhere (Settings → Account) opens Passwords.
    let unlock = lib.update(cx, |l, cx| l.unlock_account(password(ACCOUNT_PASSWORD), cx));
    assert_eq!(finish(unlock, cx), Ok(()));
    assert_eq!(vault_state(&app, cx), LockState::Unlocked);
}

#[gpui_kit::test]
fn vault_linked_sign_out_locks_and_signing_back_in_reopens(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    set_up_under_account(&app, cx);
    add_login(&app, "Mail", cx);
    sign_out(&app, cx);
    assert_eq!(vault_state(&app, cx), LockState::Locked);
    assert!(!account_open(&app, cx));
    let lib = library(&app, cx);
    assert!(cx.read(|cx| lib.read(cx).vault().items().is_empty()));
    dispatch(&app, cx, ottero_desktop::actions::GoPasswords);
    assert_eq!(
        vault_screen(&app, cx),
        VaultScreen::SignedOut(ACCOUNT_EMAIL.into())
    );
    assert!(has_label(&app, cx, "Sign in…"));
    assert_eq!(
        opener(&app, cx),
        Some(Opener::Account),
        "still linked, nothing removed"
    );

    sign_in_with_password(&app, cx);
    assert!(account_open(&app, cx));
    assert_eq!(
        vault_state(&app, cx),
        LockState::Unlocked,
        "signing in opens it"
    );
    assert_eq!(vault_titles(&app, cx), ["Mail"]);
}

#[gpui_kit::test]
fn vault_another_account_cannot_open_it_and_the_recovery_key_moves_it(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    let recovery = set_up_under_account(&app, cx);
    add_login(&app, "Mail", cx);
    sign_out(&app, cx);

    sign_up_as(&app, &fake, "bea@example.com", cx);
    assert_eq!(
        vault_state(&app, cx),
        LockState::Locked,
        "B's key isn't tried on A's vault"
    );
    dispatch(&app, cx, ottero_desktop::actions::GoPasswords);
    assert_eq!(
        vault_screen(&app, cx),
        VaultScreen::OtherAccount(ACCOUNT_EMAIL.into(), "bea@example.com".into())
    );
    assert!(has_label(&app, cx, "Account settings…"));
    assert!(
        !element_present(&app, cx, "vault-account-password"),
        "B's password can't help"
    );
    let lib = library(&app, cx);
    assert_eq!(
        cx.read(|cx| lib.read(cx).vault_linked_email().map(str::to_owned)),
        Some(ACCOUNT_EMAIL.to_owned()),
        "nothing changed or deleted"
    );
    // The recovery dialog is one click away.
    click(&app, cx, "vault-forgot");
    assert!(dialog_open(&app, cx));
    press(&app, cx, "escape");

    // A wrong recovery key changes nothing; the right one moves it to B.
    let wrong = lib.update(cx, |l, cx| {
        l.recover_vault("AAAA-AAAA-AAAA-AAAA-AAAA-AAAA-AAAA-AAAA", None, cx)
    });
    assert_eq!(finish(wrong, cx).ok(), Some(false));
    assert_eq!(vault_state(&app, cx), LockState::Locked);
    let right = lib.update(cx, |l, cx| l.recover_vault(&recovery, None, cx));
    assert_eq!(finish(right, cx).ok(), Some(true));
    assert_eq!(vault_state(&app, cx), LockState::Unlocked);
    assert_eq!(vault_titles(&app, cx), ["Mail"]);
    assert_eq!(
        cx.read(|cx| lib.read(cx).vault_linked_email().map(str::to_owned)),
        Some("bea@example.com".to_owned())
    );
}

#[gpui_kit::test]
fn vault_recovery_key_opens_a_linked_vault_while_signed_out(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    let recovery = set_up_under_account(&app, cx);
    add_login(&app, "Mail", cx);
    sign_out(&app, cx);
    let lib = library(&app, cx);
    lib.update(cx, |l, _| {
        l.set_kdf_params_for_tests(ottero_crypto::KdfParams::MINIMUM)
    });
    // Without an open account, recovery needs a new master password.
    let no_password = lib.update(cx, |l, cx| l.recover_vault(&recovery, None, cx));
    assert!(finish(no_password, cx).is_err());
    let recover = lib.update(cx, |l, cx| {
        l.recover_vault(&recovery, Some(password("a whole new master 7")), cx)
    });
    assert_eq!(finish(recover, cx).ok(), Some(true));
    assert_eq!(vault_titles(&app, cx), ["Mail"]);
    assert_eq!(
        opener(&app, cx),
        Some(Opener::MasterPassword),
        "a local vault again"
    );
    lib.update(cx, |l, cx| l.lock_vault(cx));
    let unlock = lib.update(cx, |l, cx| {
        l.unlock_vault(password("a whole new master 7"), cx)
    });
    assert_eq!(finish(unlock, cx).ok(), Some(true));
}

#[gpui_kit::test]
fn vault_late_link_after_an_account_switch_is_dropped(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    local_vault_with_items(&app, cx);
    sign_up_through_the_ui(&app, &fake, cx);
    let lib = library(&app, cx);
    // A's link is still checking the master password when B signs in here.
    let link = lib.update(cx, |l, cx| {
        let task = l.link_vault_to_account(password(MASTER), cx);
        l.account_sign_out(cx).expect("signed out");
        l.set_account_session(other_account_signed_in());
        task
    });
    let (result, recovery) = finish(link, cx).expect("link");
    assert_eq!(result, ottero_desktop::vault::LinkResult::Stale);
    assert!(recovery.is_none());
    assert_eq!(
        opener(&app, cx),
        Some(Opener::MasterPassword),
        "not linked to anyone"
    );
    assert_eq!(vault_state(&app, cx), LockState::Locked);
    let unlock = lib.update(cx, |l, cx| l.unlock_vault(password(MASTER), cx));
    assert_eq!(
        finish(unlock, cx).ok(),
        Some(true),
        "the master password still works"
    );
    assert_eq!(vault_titles(&app, cx), ["Bank", "GitHub"]);
}

#[gpui_kit::test]
fn vault_late_account_unlock_for_a_previous_account_opens_nothing(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    set_up_under_account(&app, cx);
    let lib = library(&app, cx);
    lib.update(cx, |l, cx| l.lock_vault(cx));
    assert!(!account_open(&app, cx));
    let unlock = lib.update(cx, |l, cx| {
        let task = l.unlock_account(password(ACCOUNT_PASSWORD), cx);
        l.set_account_session(other_account_signed_in());
        task
    });
    assert!(finish(unlock, cx).is_err());
    assert_eq!(
        vault_state(&app, cx),
        LockState::Locked,
        "A's late key never opens the vault"
    );
}

#[gpui_kit::test]
fn vault_linked_backup_restores_with_the_account(cx: &mut TestAppContext) {
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    set_up_under_account(&app, cx);
    add_login(&app, "Mail", cx);
    let lib = library(&app, cx);
    let path = app.paths.data_dir.join("linked.otvault");
    cx.read(|cx| lib.read(cx).export_vault(&path))
        .expect("export");
    // Delete the item, then restore the backup with the open account.
    let id = cx.read(|cx| lib.read(cx).vault().items()[0].id());
    lib.update(cx, |l, cx| l.delete_vault_items(&[id], cx))
        .expect("delete");
    let restore = lib.update(cx, |l, cx| {
        l.import_vault_archive(
            path.clone(),
            ottero_desktop::library::ArchiveSecret::Account,
            cx,
        )
    });
    assert_eq!(finish(restore, cx).ok().flatten(), Some(1));
    assert_eq!(vault_titles(&app, cx), ["Mail"]);
}

#[gpui_kit::test]
fn account_key_auto_locks_after_idle_and_sleep_without_a_vault(cx: &mut TestAppContext) {
    use ottero_core::vault::lock::AutoLock;

    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    assert_eq!(vault_state(&app, cx), LockState::Uninitialized);
    assert!(account_open(&app, cx));

    let lib = library(&app, cx);
    cx.update(|cx| {
        ottero_desktop::vault_prefs::update(cx, |p| p.auto_lock = AutoLock::OneMinute);
        lib.update(cx, |l, _| {
            l.backdate_vault_activity_for_tests(Duration::from_secs(120))
        });
    });
    cx.executor().advance_clock(Duration::from_secs(11));
    cx.run_until_parked();
    assert!(
        !account_open(&app, cx),
        "idle auto-lock should close an open account key even with no vault"
    );

    // Unlock again: opening the account key resets the idle clock and sleep still locks it.
    let unlock = lib.update(cx, |l, cx| l.unlock_account(password(ACCOUNT_PASSWORD), cx));
    assert!(finish(unlock, cx).is_ok());
    assert!(account_open(&app, cx));

    let passwords = cx.read(|cx| app.shell.read(cx).passwords().clone());
    cx.update(|cx| passwords.update(cx, |p, _| p.pretend_slept_for_tests()));
    cx.executor().advance_clock(Duration::from_secs(11));
    cx.run_until_parked();
    assert!(
        !account_open(&app, cx),
        "waking from sleep should close an open account key even with no vault"
    );
}

#[gpui_kit::test]
fn signed_in_library_mutations_enqueue_sync_outbox_while_local_only_does_not(
    cx: &mut TestAppContext,
) {
    // 1. Local-only user: no outbox rows are created.
    let local = launch(cx);
    let local_lib = library(&local, cx);
    cx.update(|cx| {
        local_lib.update(cx, |l, cx| {
            l.create_note("Local note", cx).expect("note");
            l.create_task("Local task", None, cx).expect("task");
        });
    });
    assert_eq!(
        cx.read(|cx| local_lib.read(cx).pending_sync_count()),
        0,
        "local-only users must not accumulate sync outbox rows"
    );

    // 2. Signed-in user: Note, Task, and Task updates enqueue automatically.
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    let lib = library(&app, cx);
    assert_eq!(cx.read(|cx| lib.read(cx).pending_sync_count()), 0);

    cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.create_note("Synced note", cx).expect("note");
            let tid = l.create_task("Synced task", None, cx).expect("task");
            l.set_task_priority(tid, ottero_core::Priority::High, cx)
                .expect("priority");
        });
    });
    assert_eq!(
        cx.read(|cx| lib.read(cx).pending_sync_count()),
        3,
        "signed-in mutations should enqueue into sync_outbox"
    );

    // 3. Signing out clears local sync state and outbox so another account never inherits it.
    cx.update(|cx| {
        lib.update(cx, |l, cx| l.account_sign_out(cx).expect("sign out"));
    });
    assert_eq!(
        cx.read(|cx| lib.read(cx).pending_sync_count()),
        0,
        "sign-out must clear sync_outbox"
    );
}
