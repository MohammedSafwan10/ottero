use super::*;
use crate::account::*;
use crate::core_ui::*;
use crate::notes_tasks::*;

// ---- Two-step verification ----------------------------------------------

use ottero_desktop::views::account::{TwoStepDialog, TwoStepStep};

pub(crate) fn two_step_dialog(app: &App, cx: &mut TestAppContext) -> Option<Entity<TwoStepDialog>> {
    let view = account_view(app, cx);
    cx.read(|cx| view.read(cx).two_step_dialog())
}

pub(crate) fn dialog_step(app: &App, cx: &mut TestAppContext) -> Option<TwoStepStep> {
    let dialog = two_step_dialog(app, cx)?;
    Some(cx.read(|cx| dialog.read(cx).step()))
}

/// The code an authenticator app shows now for an `otpauth://` link.
pub(crate) fn authenticator_code(uri: &str) -> String {
    let now = ottero_core::clock::now().as_second().cast_unsigned();
    ottero_core::vault::totp::parse(uri)
        .expect("otpauth link")
        .code_at(now)
        .code
}

pub(crate) fn current_token(app: &App, cx: &mut TestAppContext) -> String {
    let lib = library(app, cx);
    cx.read(|cx| {
        lib.read(cx)
            .account()
            .current_token()
            .map(|t| t.expose().to_owned())
            .expect("token")
    })
}

pub(crate) fn password(text: &str) -> ottero_crypto::SecretString {
    ottero_crypto::SecretString::from(text.to_owned())
}

/// Turns two-step on through the `Library` (the UI path has its own test).
/// Returns the setup link and the backup codes.
pub(crate) fn turn_on_two_step(app: &App, cx: &mut TestAppContext) -> (String, Vec<String>) {
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

pub(crate) fn sign_in_with_password(app: &App, cx: &mut TestAppContext) {
    dispatch(app, cx, ottero_desktop::actions::SignInToCloud);
    assert_eq!(account_screen(app, cx), Screen::SignIn);
    // The email is kept and this device has the Secret Key: only the password.
    assert!(!shows_secret_key_field(app, cx));
    type_text(app, cx, ACCOUNT_PASSWORD);
    press(app, cx, "enter");
}

pub(crate) fn sign_out(app: &App, cx: &mut TestAppContext) {
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
