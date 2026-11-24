//! Ottero desktop app.
//!
//! Startup order matters: GPUI Kit init → design system (fonts, themes,
//! appearance) → key bindings and commands → open the database → open the
//! window. [`start`] does all of it and is shared by `main` and the UI tests,
//! so tests exercise the real app.

pub mod account;
pub mod actions;
pub mod focus;
pub mod holidays;
pub mod library;
pub mod markdown_io;
pub mod nav;
pub mod navigation;
pub mod paths;
#[cfg(feature = "perf")]
pub mod perf;
pub mod private_file;
pub mod shell;
pub mod sync;
pub mod ui_state;
pub mod updates;
pub mod vault;
pub mod vault_prefs;
pub mod views;

use anyhow::{Context as _, Result};
use gpui_kit::component::TitleBar;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, WindowBounds, WindowOptions, px, size,
};
use ottero_storage::Store;

use crate::library::Library;
use crate::paths::AppPaths;
use crate::shell::AppShell;

pub fn run() -> Result<()> {
    let paths = AppPaths::resolve()?;
    let _instance = updates::InstanceLock::acquire(&paths.data_dir)?;
    // Open the database before the event loop so a broken data folder fails
    // fast with a readable message instead of an empty window.
    let store = Store::open(&paths.database)
        .with_context(|| format!("could not open {}", paths.database.display()))?;

    gpui_kit::application()
        .with_assets(ottero_ui::assets::OtteroAssets)
        .run(move |cx| match start(&paths, store, cx) {
            Ok(_) => cx.activate(true),
            Err(err) => {
                eprintln!("ottero: failed to open window: {err:#}");
                cx.quit();
            }
        });
    Ok(())
}

/// Initializes everything and opens the main window.
pub fn start(
    paths: &AppPaths,
    store: Store,
    cx: &mut App,
) -> Result<(AnyWindowHandle, Entity<AppShell>)> {
    let api = std::sync::Arc::from(account::client::HttpAccountClient::from_env());
    start_inner(paths, store, api, true, cx)
}

/// [`start`] with a given Ottero Cloud server, so tests never reach a real
/// one (a local dev server may be listening on the default debug URL).
pub fn start_with_account_api(
    paths: &AppPaths,
    store: Store,
    account_api: std::sync::Arc<dyn account::client::AccountApi>,
    cx: &mut App,
) -> Result<(AnyWindowHandle, Entity<AppShell>)> {
    start_inner(paths, store, account_api, false, cx)
}

fn start_inner(
    paths: &AppPaths,
    store: Store,
    account_api: std::sync::Arc<dyn account::client::AccountApi>,
    auto_sync: bool,
    cx: &mut App,
) -> Result<(AnyWindowHandle, Entity<AppShell>)> {
    gpui_kit::init(cx);
    ottero_ui::init(paths.appearance.clone(), cx);
    ui_state::initialize_navigation(
        &paths.data_dir,
        ottero_ui::appearance::settings(cx).developer_tools,
        paths.existing_database,
    );
    vault_prefs::init(&paths.data_dir, cx);
    // The note editor's own keys first: ours are bound after, so they win on a tie.
    zorite_editor::bind_keys(cx);
    actions::bind(cx);
    actions::set_menus(cx);
    shell::register_commands(cx);
    holidays::ensure_countries(cx);

    let library = cx.new(|cx| {
        let mut lib = Library::new(store);
        lib.set_archive_dir(paths.data_dir.join("vault-archives"));
        lib.set_account_session(account::AccountSession::new(
            account::store::AccountFile::open(paths.data_dir.join("account.json")),
            account_api,
        ));
        if auto_sync {
            lib.enable_auto_sync(cx);
        }
        lib
    });
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1200.), px(780.)),
            cx,
        ))),
        window_min_size: Some(size(px(720.), px(480.))),
        // Same identifier as the Mac bundle ID, which stays the pre-rename
        // one so installed apps keep updating (ADR 0009).
        app_id: Some("com.nexdark.calmfold".into()),
        ..TitleBar::window_options()
    };
    let paths_data_dir = paths.data_dir.clone();
    let (window, shell) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| AppShell::new(library, paths, window, cx))
    })?;
    AppShell::install(&shell, window, cx);
    // After the window: an update found at startup shows its notice there.
    updates::init(&paths_data_dir, cx);
    {
        let shell = shell.downgrade();
        window.update(cx, |_, window, cx| {
            window.on_window_should_close(cx, move |_, cx| {
                _ = shell.update(cx, |shell, cx| shell.save_everything(cx));
                true
            });
        })?;
    }

    #[cfg(feature = "perf")]
    perf::install(window, cx);

    // Debug and perf builds only: start on a given screen, for screenshot and perf scripts.
    #[cfg(any(debug_assertions, feature = "perf"))]
    if let Some(route) = std::env::var("OTTERO_DEV_ROUTE")
        .ok()
        .and_then(|r| views::Route::parse(&r))
    {
        window.update(cx, |_, window, cx| {
            shell.update(cx, |s, cx| s.navigate(route, window, cx))
        })?;
        // Existing review hook: open the Workspace settings directly.
        if route == views::Route::Settings
            && std::env::var("OTTERO_DEV_SETTINGS")
                .is_ok_and(|v| v.eq_ignore_ascii_case("workspace"))
        {
            window.update(cx, |_, w, cx| {
                w.dispatch_action(Box::new(actions::CustomizeSidebar), cx)
            })?;
        }
        if route == views::Route::Settings
            && std::env::var("OTTERO_DEV_SETTINGS").is_ok_and(|v| v == "changelog")
        {
            window.update(cx, |_, w, cx| {
                w.dispatch_action(Box::new(actions::ShowChangelog), cx)
            })?;
        }
        // OTTERO_DEV_SOURCE=1 with route=notes: open the first note as Markdown source.
        if route == views::Route::Notes && std::env::var_os("OTTERO_DEV_SOURCE").is_some() {
            window.update(cx, |_, window, cx| {
                let notes = shell.read(cx).notes().clone();
                notes.update(cx, |n, cx| {
                    n.ensure_selection(window, cx);
                    n.toggle_source_mode(window, cx);
                });
            })?;
        }
        // OTTERO_DEV_CALENDAR=month|week|day|agenda: open the calendar in that view.
        if let Ok(name) = std::env::var("OTTERO_DEV_CALENDAR") {
            let mode = views::calendar_layout::CalendarMode::OPTIONS
                .into_iter()
                .find(|(_, label)| label.eq_ignore_ascii_case(name.trim()))
                .map(|(mode, _)| mode);
            if let Some(mode) = mode {
                let calendar = shell.read(cx).calendar().clone();
                calendar.update(cx, |c, cx| c.set_mode(mode, cx));
            }
        }
        // OTTERO_DEV_HABITS=insights|archived: open that Habits tab.
        if let Ok(name) = std::env::var("OTTERO_DEV_HABITS") {
            use views::habits::HabitTab;
            let tab = match name.trim() {
                "insights" => Some(HabitTab::Insights),
                "archived" => Some(HabitTab::Archived),
                _ => None,
            };
            if let Some(tab) = tab {
                let habits = shell.read(cx).habits().clone();
                habits.update(cx, |h, cx| h.set_tab(tab, cx));
            }
        }
        // OTTERO_DEV_VAULT=<password> (debug builds only): set up or unlock
        // the vault with that password and add sample items to an empty one.
        #[cfg(debug_assertions)]
        if let Ok(password) = std::env::var("OTTERO_DEV_VAULT") {
            dev_vault(window, shell.clone(), password, cx);
        }
        // OTTERO_DEV_ACCOUNT=<password> (debug builds only): unlock the
        // signed-in Ottero account at startup, for screenshots of the
        // Passwords link and account screens. The password is never logged.
        #[cfg(debug_assertions)]
        if let Ok(password) = std::env::var("OTTERO_DEV_ACCOUNT") {
            let library = shell.read(cx).library().clone();
            library
                .update(cx, |lib, cx| {
                    lib.unlock_account(ottero_crypto::SecretString::from(password), cx)
                })
                .detach();
        }
        // OTTERO_DEV_NOTE=<title>: open that note with the caret at the end of its body.
        if let Ok(title) = std::env::var("OTTERO_DEV_NOTE") {
            window.update(cx, |_, window, cx| {
                let notes = shell.read(cx).notes().clone();
                notes.update(cx, |n, cx| n.open_note_titled_at_end(&title, window, cx));
            })?;
        }
    }
    Ok((window, shell))
}

/// Debug-only: unlock (or create) the vault and fill an empty one with
/// sample items, for screenshots and manual testing.
#[cfg(debug_assertions)]
fn dev_vault(window: AnyWindowHandle, shell: Entity<AppShell>, password: String, cx: &mut App) {
    let library = shell.read(cx).library().clone();
    use ottero_core::vault::lock::LockState;
    use ottero_core::vault::{ItemFields, ItemKind};
    use ottero_crypto::SecretString;
    let fresh = library.read(cx).vault_state() == LockState::Uninitialized;
    let (setup, unlock) = library.update(cx, |lib, cx| {
        if fresh {
            (
                Some(lib.set_up_vault(SecretString::from(password), cx)),
                None,
            )
        } else {
            (
                None,
                Some(lib.unlock_vault(SecretString::from(password), cx)),
            )
        }
    });
    cx.spawn(async move |cx| {
        let opened = match (setup, unlock) {
            (Some(t), _) => t.await.is_ok(),
            (_, Some(t)) => matches!(t.await, Ok(true)),
            _ => false,
        };
        if !opened {
            eprintln!("ottero: OTTERO_DEV_VAULT couldn't open the vault");
            return;
        }
        library.update(cx, |lib, cx| {
            if lib.vault().count(vault::VaultFilter::All) > 0 {
                return;
            }
            let login =
                |title: &str, user: &str, pw: &str, url: &str, totp: &str, tags: &[&str]| {
                    ItemFields {
                        title: title.into(),
                        username: user.into(),
                        password: pw.into(),
                        urls: vec![url.into()],
                        totp: totp.into(),
                        tags: tags.iter().map(|t| (*t).to_owned()).collect(),
                        ..ItemFields::new(ItemKind::Login)
                    }
                };
            let rows = vec![
                (
                    login(
                        "GitHub",
                        "sam@example.com",
                        "Tangy-Velvet-Orbit-42!",
                        "https://github.com/login",
                        "JBSWY3DPEHPK3PXP",
                        &["work"],
                    ),
                    true,
                ),
                (
                    login(
                        "Gmail",
                        "sam@example.com",
                        "hunter2",
                        "https://mail.google.com",
                        "",
                        &[],
                    ),
                    false,
                ),
                (
                    login(
                        "Amazon",
                        "sam@example.com",
                        "Tangy-Velvet-Orbit-42!",
                        "https://amazon.com",
                        "",
                        &["shopping"],
                    ),
                    false,
                ),
                (
                    login(
                        "Netflix",
                        "sam.family",
                        "q7#Lp2!xVz9rT4mK",
                        "https://netflix.com",
                        "",
                        &["home"],
                    ),
                    true,
                ),
                (
                    login(
                        "Figma",
                        "sam@studio.io",
                        "Walrus-Quiet-Ember-Trumpet-7",
                        "https://figma.com",
                        "",
                        &["work"],
                    ),
                    false,
                ),
                (
                    login(
                        "Home router",
                        "admin",
                        "password123",
                        "192.168.1.1",
                        "",
                        &["home"],
                    ),
                    false,
                ),
                (
                    ItemFields {
                        title: "Travel Visa".into(),
                        cardholder: "SAM RIVERA".into(),
                        number: "4111 1111 1111 4242".into(),
                        expiry: "08/29".into(),
                        cvv: "123".into(),
                        ..ItemFields::new(ItemKind::Card)
                    },
                    false,
                ),
                (
                    ItemFields {
                        title: "Home Wi-Fi".into(),
                        notes: "Network: Rivera-5G\nPassword: plum orbit candle".into(),
                        ..ItemFields::new(ItemKind::Note)
                    },
                    false,
                ),
            ];
            if let Err(err) = lib.import_vault_items(rows, cx) {
                eprintln!("ottero: sample vault items failed: {err:#}");
            }
        });
        // OTTERO_DEV_VAULT_ITEM=<title>: open that item.
        if let Ok(title) = std::env::var("OTTERO_DEV_VAULT_ITEM") {
            // After the list has refreshed from the import above.
            cx.background_executor()
                .timer(std::time::Duration::from_millis(300))
                .await;
            _ = window.update(cx, |_, window, cx| {
                if shell.read(cx).route() == views::Route::Developer {
                    let developer = shell.read(cx).developer().clone();
                    developer.update(cx, |d, cx| d.select_for_dev(&title, window, cx));
                    return;
                }
                let passwords = shell.read(cx).passwords().clone();
                passwords.update(cx, |p, cx| p.select_for_dev(&title, window, cx));
            });
        }
    })
    .detach();
}
