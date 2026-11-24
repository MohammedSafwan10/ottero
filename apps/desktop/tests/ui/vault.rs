use super::*;
use crate::core_ui::*;
use crate::notes_tasks::*;

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
pub(crate) fn unlocked_vault(cx: &mut TestAppContext) -> App {
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

pub(crate) fn element_present(app: &App, cx: &mut TestAppContext, id: &'static str) -> bool {
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

pub(crate) fn click_placeholder(app: &App, cx: &mut TestAppContext, placeholder: &str) {
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

pub(crate) fn set_up_with_key(app: &App, cx: &mut TestAppContext) -> String {
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
