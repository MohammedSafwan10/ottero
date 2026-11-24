use super::*;
use crate::account::*;
use crate::core_ui::*;
use crate::notes_tasks::*;
use crate::two_factor::*;
use crate::vault::*;

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
/// Cloud-linked vaults go straight to Unlocked without a redundant second
/// recovery-key gate (the Account Emergency Kit covers recovery).
fn set_up_under_account(app: &App, cx: &mut TestAppContext) {
    use ottero_desktop::actions::GoPasswords;
    dispatch(app, cx, GoPasswords);
    assert_eq!(vault_screen(app, cx), VaultScreen::SetupWithAccount);
    assert!(
        !element_present(app, cx, "vault-master"),
        "no master password is asked for"
    );
    click(app, cx, "vault-create-account");
    assert_eq!(vault_state(app, cx), LockState::Unlocked);
    assert_eq!(vault_screen(app, cx), VaultScreen::Unlocked);
}

/// Explicitly generates an optional local vault recovery key (for tests that
/// exercise offline vault recovery on a Cloud-linked vault).
fn create_optional_recovery_key(app: &App, cx: &mut TestAppContext) -> String {
    let lib = library(app, cx);
    let task = lib.update(cx, |l, cx| l.new_recovery_key(cx));
    finish(task, cx).expect("recovery key").to_string()
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
    set_up_under_account(&app, cx);
    let recovery = create_optional_recovery_key(&app, cx);
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
    set_up_under_account(&app, cx);
    let recovery = create_optional_recovery_key(&app, cx);
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

    // 2. Signed-in Free user: no outbox rows accumulate without an active Cloud plan.
    let (app, fake) = launch_with_account(cx);
    sign_up_through_the_ui(&app, &fake, cx);
    let lib = library(&app, cx);
    cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.create_note("Free note", cx).expect("note");
        });
    });
    assert_eq!(
        cx.read(|cx| lib.read(cx).pending_sync_count()),
        0,
        "signed-in Free users must not accumulate unbounded sync_outbox rows"
    );

    // 3. Signed-in Cloud/Owner user: Note, Task, and Task updates enqueue automatically.
    fake.grant_owner(ACCOUNT_EMAIL);
    cx.update(|cx| lib.update(cx, |l, cx| l.refresh_account(cx)));
    cx.run_until_parked();

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
        "signed-in Cloud mutations should enqueue into sync_outbox"
    );

    // 4. Signing out clears local sync state and outbox so another account never inherits it.
    cx.update(|cx| {
        lib.update(cx, |l, cx| l.account_sign_out(cx).expect("sign out"));
    });
    assert_eq!(
        cx.read(|cx| lib.read(cx).pending_sync_count()),
        0,
        "sign-out must clear sync_outbox"
    );
}
