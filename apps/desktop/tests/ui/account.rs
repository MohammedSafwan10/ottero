use super::*;
use crate::core_ui::*;
use crate::notes_tasks::*;

// ---- Ottero Cloud account ------------------------------------------------

pub(crate) use ottero_desktop::account::fake::FakeAccountApi;
pub(crate) use ottero_desktop::views::account::{AccountView, Screen};

pub(crate) const ACCOUNT_EMAIL: &str = "sam@example.com";
pub(crate) const ACCOUNT_PASSWORD: &str = "river-lantern-42";

/// The app with an in-memory account server and cheap Argon2.
pub(crate) fn launch_with_account(
    cx: &mut TestAppContext,
) -> (App, std::sync::Arc<FakeAccountApi>) {
    let app = launch(cx);
    let fake = std::sync::Arc::new(FakeAccountApi::new());
    attach_fake(&app, &fake, cx);
    (app, fake)
}

pub(crate) fn attach_fake(
    app: &App,
    fake: &std::sync::Arc<FakeAccountApi>,
    cx: &mut TestAppContext,
) {
    let api: std::sync::Arc<dyn ottero_desktop::account::client::AccountApi> = fake.clone();
    library(app, cx).update(cx, |lib, _| {
        lib.set_account_api_for_tests(api, ottero_crypto::account::AccountKdf::INSECURE_TEST);
    });
}

pub(crate) fn account_view(app: &App, cx: &mut TestAppContext) -> Entity<AccountView> {
    cx.read(|cx| app.shell.read(cx).settings().read(cx).account().clone())
}

pub(crate) fn account_screen(app: &App, cx: &mut TestAppContext) -> Screen {
    let view = account_view(app, cx);
    cx.read(|cx| view.read(cx).screen(cx))
}

pub(crate) fn account_error(app: &App, cx: &mut TestAppContext) -> Option<String> {
    let lib = library(app, cx);
    cx.read(|cx| lib.read(cx).account().error().map(|(_, m)| m.to_owned()))
}

pub(crate) fn has_label(app: &App, cx: &mut TestAppContext, label: &str) -> bool {
    use gpui_kit::test::TestQueryExt;
    cx.update_window(app.window, |_, window, cx| {
        window.render_frame(cx);
        window.try_find_by_label(label).is_some()
    })
    .expect("window")
}

pub(crate) fn shows_secret_key_field(app: &App, cx: &mut TestAppContext) -> bool {
    let view = account_view(app, cx);
    cx.read(|cx| view.read(cx).needs_secret_key(cx))
}

/// Create account → code (one wrong try) → Emergency Kit → signed in.
/// Returns the Secret Key shown in the kit.
pub(crate) fn sign_up_through_the_ui(
    app: &App,
    fake: &FakeAccountApi,
    cx: &mut TestAppContext,
) -> String {
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

pub(crate) fn finish<T: 'static>(task: gpui_kit::Task<T>, cx: &mut TestAppContext) -> T {
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
pub(crate) fn other_account_signed_in() -> ottero_desktop::account::AccountSession {
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
