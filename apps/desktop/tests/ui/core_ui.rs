use super::*;

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

pub(crate) fn click(app: &App, cx: &mut TestAppContext, id: &'static str) {
    cx.update_window(app.window, |_, window, cx| window.click(id, cx))
        .expect("window");
    cx.run_until_parked();
}

pub(crate) fn dialog_open(app: &App, cx: &mut TestAppContext) -> bool {
    cx.update_window(app.window, |_, window, cx| {
        gpui_kit::component::WindowExt::has_active_dialog(window, cx)
    })
    .expect("window")
}

pub(crate) fn create_and_open_note(
    app: &App,
    cx: &mut TestAppContext,
    title: &str,
) -> ottero_core::NoteId {
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

pub(crate) fn note_state(
    app: &App,
    cx: &mut TestAppContext,
    id: ottero_core::NoteId,
) -> Option<bool> {
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

pub(crate) fn all_note_titles(app: &App, cx: &mut TestAppContext) -> Vec<String> {
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
            assert!(
                texts
                    .iter()
                    .any(|t| t.text() == "Ottero Cloud, 2FA, and Encrypted Sync")
            );
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
pub(crate) fn click_label(app: &App, cx: &mut TestAppContext, label: &str) {
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
