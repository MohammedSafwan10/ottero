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

mod account;
mod account_vault_sync;
mod core_ui;
mod editor;
mod notes_tasks;
mod two_factor;
mod vault;
