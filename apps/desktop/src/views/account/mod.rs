//! Settings → Account: Ottero Cloud sign-up, code entry, Emergency Kit,
//! sign-in (with the Secret Key on a new device, and the authenticator or
//! backup code when two-step verification is on) and the signed-in page.
//! Turning two-step verification on or off and new backup codes run in
//! [`TwoStepDialog`].
//!
//! The screen follows `Library::account().state()` (the state machine in
//! `ottero_core::account`); this view only owns the form fields and which
//! form a signed-out user opened ([`Mode`]). Every step that waits on the
//! network or Argon2 shows the spinner on its own button; errors sit next to
//! the field they are about. Enter submits, Tab moves between fields, Escape
//! goes back.
//!
//! The page is built as Settings groups ([`page`]) so it shares the look,
//! sidebar sections and search of the other Settings pages.

mod auth_flows;
mod signed_in;
mod two_step_dialog;
mod widgets;

pub use two_step_dialog::{TwoStepDialog, TwoStepPurpose, TwoStepStep};

use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, SharedString, Styled as _, Subscription,
    Task as GpuiTask, Window, div,
};
use ottero_core::account::{AccountState, Flow, SecretKeyPresence, check_password};
use ottero_crypto::{SecretString, Zeroizing};

use crate::account::Field;
use crate::account::client::SecondFactor;
use crate::library::{Library, LibraryEvent};
use crate::views::vault_item::copy_field;
use two_step_dialog::open_two_step_dialog;
use widgets::{open_password_prompt, save_kit};

/// Which form a signed-out user is looking at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Intro,
    SignUp,
    SignIn,
}

/// What the screen shows, derived from the account state and [`Mode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Intro,
    SignUp,
    SignIn,
    Code,
    /// The authenticator (or backup) code after the password.
    TwoStep,
    EmergencyKit,
    SignedIn,
}

pub struct AccountView {
    library: Entity<Library>,
    mode: Mode,
    email: Entity<InputState>,
    password: Entity<InputState>,
    confirm: Entity<InputState>,
    secret_key: Entity<InputState>,
    code: Entity<InputState>,
    two_step_code: Entity<InputState>,
    backup_code: Entity<InputState>,
    /// The second step asks for a backup code instead of the app's code.
    use_backup: bool,
    unlock: Entity<InputState>,
    kit_saved: bool,
    reveal_secret_key: bool,
    unlock_error: Option<SharedString>,
    /// The two-step dialog last opened (tests and focus checks read it).
    two_step_dialog: Option<gpui_kit::WeakEntity<TwoStepDialog>>,
    last_screen: Screen,
    _tick: Option<GpuiTask<()>>,
    _subscriptions: Vec<Subscription>,
}

impl AccountView {
    pub fn new(library: Entity<Library>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let plain = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let masked = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder(placeholder)
            })
        };
        let email = plain("name@example.com", window, cx);
        let password = masked("At least 10 characters", window, cx);
        let confirm = masked("Type it again", window, cx);
        let secret_key = plain("O1-XXXXXX-XXXXX-XXXXX-XXXXX-XXXXX", window, cx);
        let code = plain("123 456", window, cx);
        let two_step_code = plain("123 456", window, cx);
        let backup_code = plain("Ab12c-De34f", window, cx);
        let unlock = masked("Password", window, cx);

        let mut subscriptions =
            vec![
                cx.subscribe_in(&library, window, |this, _, event, window, cx| {
                    if *event == LibraryEvent::AccountChanged {
                        this.on_account_changed(window, cx);
                    }
                }),
            ];
        for input in [&email, &password, &confirm, &secret_key] {
            subscriptions.push(
                cx.subscribe_in(input, window, |this, _, event, window, cx| match event {
                    InputEvent::Change => {
                        this.library.update(cx, Library::clear_account_error);
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => this.submit_form(window, cx),
                    _ => {}
                }),
            );
        }
        subscriptions.push(cx.subscribe_in(
            &code,
            window,
            |this, _, event, window, cx| match event {
                InputEvent::Change => this.code_changed(window, cx),
                InputEvent::PressEnter { .. } => this.verify(cx),
                _ => {}
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &two_step_code,
            window,
            |this, _, event, window, cx| match event {
                InputEvent::Change => this.two_step_code_changed(window, cx),
                InputEvent::PressEnter { .. } => this.verify_two_step(cx),
                _ => {}
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &backup_code,
            window,
            |this, _, event, _, cx| match event {
                InputEvent::Change => {
                    this.library.update(cx, Library::clear_account_error);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.verify_two_step(cx),
                _ => {}
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &unlock,
            window,
            |this, _, event, window, cx| match event {
                InputEvent::Change => {
                    this.unlock_error = None;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.unlock_account(window, cx),
                _ => {}
            },
        ));

        let this = Self {
            library: library.clone(),
            mode: Mode::Intro,
            email,
            password,
            confirm,
            secret_key,
            code,
            two_step_code,
            backup_code,
            use_backup: false,
            unlock,
            kit_saved: false,
            reveal_secret_key: false,
            unlock_error: None,
            two_step_dialog: None,
            last_screen: Screen::Intro,
            _tick: None,
            _subscriptions: subscriptions,
        };
        library.update(cx, Library::refresh_account);
        this
    }

    #[must_use]
    pub fn library(&self) -> &Entity<Library> {
        &self.library
    }

    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    #[must_use]
    pub fn screen(&self, cx: &App) -> Screen {
        match self.library.read(cx).account().state() {
            AccountState::SignedOut => match self.mode {
                Mode::Intro => Screen::Intro,
                Mode::SignUp => Screen::SignUp,
                Mode::SignIn => Screen::SignIn,
            },
            AccountState::SigningUp { .. } => Screen::SignUp,
            AccountState::Error { flow, .. } => match flow {
                Flow::SignUp => Screen::SignUp,
                Flow::SignIn => Screen::SignIn,
            },
            AccountState::SigningIn { .. } | AccountState::NeedsSecretKey { .. } => Screen::SignIn,
            AccountState::AwaitingCode { .. } => Screen::Code,
            AccountState::TwoStep { .. } => Screen::TwoStep,
            AccountState::EmergencyKit { .. } => Screen::EmergencyKit,
            AccountState::SignedIn { .. } => Screen::SignedIn,
        }
    }

    /// Opens a form (from a button or a command).
    pub fn open(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if self.library.read(cx).account().state().is_signed_in() {
            self.library.update(cx, Library::refresh_account);
            return;
        }
        if matches!(
            self.screen(cx),
            Screen::Code | Screen::TwoStep | Screen::EmergencyKit
        ) {
            return;
        }
        self.library.update(cx, Library::cancel_account_flow);
        self.mode = mode;
        self.focus_for(self.screen(cx), window, cx);
        cx.notify();
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.screen(cx) {
            Screen::SignUp | Screen::SignIn if self.library.read(cx).account().is_busy() => {}
            Screen::SignUp | Screen::SignIn => {
                self.library.update(cx, Library::cancel_account_flow);
                self.mode = Mode::Intro;
                self.clear_secrets(window, cx);
            }
            Screen::Code => self.change_email(window, cx),
            Screen::TwoStep if self.library.read(cx).account().code_busy() => {}
            Screen::TwoStep => {
                // Back to the password, email kept.
                self.library.update(cx, Library::cancel_account_flow);
                self.mode = Mode::SignIn;
                self.focus_for(Screen::SignIn, window, cx);
            }
            Screen::Intro | Screen::EmergencyKit | Screen::SignedIn => {}
        }
        cx.notify();
    }

    fn change_email(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.library.update(cx, Library::cancel_account_flow);
        self.mode = Mode::SignUp;
        self.code.update(cx, |i, cx| i.set_value("", window, cx));
        self.focus_for(Screen::SignUp, window, cx);
        cx.notify();
    }

    fn clear_secrets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [
            &self.password,
            &self.confirm,
            &self.secret_key,
            &self.code,
            &self.two_step_code,
            &self.backup_code,
            &self.unlock,
        ] {
            input.update(cx, |i, cx| i.set_value("", window, cx));
        }
    }

    fn focus_for(&self, screen: Screen, window: &mut Window, cx: &mut Context<Self>) {
        let target = match screen {
            Screen::SignUp | Screen::SignIn => {
                if self.email.read(cx).value().is_empty() {
                    &self.email
                } else if screen == Screen::SignIn
                    && self.needs_secret_key(cx)
                    && !self.password.read(cx).value().is_empty()
                {
                    &self.secret_key
                } else {
                    &self.password
                }
            }
            Screen::Code => &self.code,
            Screen::TwoStep if self.use_backup => &self.backup_code,
            Screen::TwoStep => &self.two_step_code,
            _ => return,
        };
        target.update(cx, |i, cx| i.focus(window, cx));
    }

    fn on_account_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let screen = self.screen(cx);
        let account = self.library.read(cx).account();
        if matches!(account.state(), AccountState::NeedsSecretKey { .. }) {
            self.secret_key.update(cx, |i, cx| i.focus(window, cx));
        }
        if screen != self.last_screen {
            match screen {
                Screen::Code => {
                    self.code.update(cx, |i, cx| i.set_value("", window, cx));
                    self.focus_for(Screen::Code, window, cx);
                }
                Screen::TwoStep => {
                    // The password did its part; a timed-out attempt asks for it again.
                    self.use_backup = false;
                    for input in [&self.password, &self.two_step_code, &self.backup_code] {
                        input.update(cx, |i, cx| i.set_value("", window, cx));
                    }
                    self.focus_for(Screen::TwoStep, window, cx);
                }
                Screen::EmergencyKit => {
                    self.kit_saved = false;
                    self.password
                        .update(cx, |i, cx| i.set_value("", window, cx));
                    self.confirm.update(cx, |i, cx| i.set_value("", window, cx));
                }
                Screen::SignedIn => {
                    self.mode = Mode::Intro;
                    self.clear_secrets(window, cx);
                }
                Screen::Intro => {
                    self.reveal_secret_key = false;
                    self.clear_secrets(window, cx);
                }
                Screen::SignUp | Screen::SignIn => {
                    if self.last_screen == Screen::TwoStep {
                        self.focus_for(screen, window, cx);
                    }
                }
            }
            self.last_screen = screen;
        }
        if !self.library.read(cx).account().is_unlocked() {
            self.reveal_secret_key = false;
        }
        // Re-render once a second while "Resend in N s" counts down.
        if screen == Screen::Code && self.library.read(cx).account().resend_wait().is_some() {
            if self._tick.is_none() {
                self._tick = Some(cx.spawn_in(window, async move |this, cx| {
                    loop {
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                        let more = this.update(cx, |this, cx| {
                            cx.notify();
                            this.library.read(cx).account().resend_wait().is_some()
                        });
                        if !matches!(more, Ok(true)) {
                            _ = this.update(cx, |this, _| this._tick = None);
                            break;
                        }
                    }
                }));
            }
        } else {
            self._tick = None;
        }
        cx.notify();
    }

    /// Whether the sign-in form shows the Secret Key field (no valid key
    /// stored on this device for the typed email).
    #[must_use]
    pub fn needs_secret_key(&self, cx: &App) -> bool {
        let account = self.library.read(cx).account();
        if matches!(account.state(), AccountState::NeedsSecretKey { .. }) {
            return true;
        }
        let email = self.email.read(cx).value();
        !email.trim().is_empty()
            && account.secret_key_presence(&email) == SecretKeyPresence::Missing
    }

    fn submit_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.screen(cx) {
            Screen::SignUp => self.create_account(cx),
            Screen::SignIn => self.sign_in(window, cx),
            _ => {}
        }
    }

    fn signup_problem(&self, cx: &App) -> Option<&'static str> {
        let password = self.password.read(cx).value();
        let confirm = self.confirm.read(cx).value();
        let email = self.email.read(cx).value();
        if password.is_empty() {
            return None;
        }
        if let Some(problem) = check_password(&password, &email).problem {
            return Some(problem);
        }
        if !confirm.is_empty() && confirm != password {
            return Some("The passwords don't match.");
        }
        None
    }

    fn create_account(&mut self, cx: &mut Context<Self>) {
        let password = self.password.read(cx).value();
        let confirm = self.confirm.read(cx).value();
        if confirm != password {
            self.library.update(cx, |lib, cx| {
                lib.set_account_error(Field::Password, "The passwords don't match.", cx);
            });
            return;
        }
        let email = self.email.read(cx).value().to_string();
        let password = SecretString::from(password.to_string());
        self.library
            .update(cx, |lib, cx| lib.account_sign_up(&email, password, cx));
    }

    fn sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let email = self.email.read(cx).value().to_string();
        let password = SecretString::from(self.password.read(cx).value().to_string());
        let key = Zeroizing::new(self.secret_key.read(cx).value().to_string());
        if self.needs_secret_key(cx)
            && key.trim().is_empty()
            && !self.password.read(cx).value().is_empty()
        {
            self.secret_key.update(cx, |i, cx| i.focus(window, cx));
        }
        self.library.update(cx, |lib, cx| {
            lib.account_sign_in(&email, password, &key, cx)
        });
    }

    /// Digits only, at most six, shown as "123 456"; the sixth digit submits.
    fn code_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.code.read(cx).value();
        let digits = ottero_core::account::code_digits(&value);
        let shown = ottero_core::account::group_code(&digits);
        if shown != value.as_ref() {
            self.code.update(cx, |i, cx| i.set_value(shown, window, cx));
        }
        self.library.update(cx, Library::clear_account_error);
        if digits.len() == 6 {
            self.verify(cx);
        }
        cx.notify();
    }

    fn verify(&mut self, cx: &mut Context<Self>) {
        let code = Zeroizing::new(self.code.read(cx).value().to_string());
        self.library
            .update(cx, |lib, cx| lib.verify_account_code(&code, cx));
    }

    /// The authenticator code field: same rules as the email code.
    fn two_step_code_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.two_step_code.read(cx).value();
        let digits = ottero_core::account::code_digits(&value);
        let shown = ottero_core::account::group_code(&digits);
        if shown != value.as_ref() {
            self.two_step_code
                .update(cx, |i, cx| i.set_value(shown, window, cx));
        }
        self.library.update(cx, Library::clear_account_error);
        if digits.len() == 6 && !self.library.read(cx).account().code_busy() {
            self.verify_two_step(cx);
        }
        cx.notify();
    }

    fn verify_two_step(&mut self, cx: &mut Context<Self>) {
        let (input, factor) = if self.use_backup {
            (&self.backup_code, SecondFactor::BackupCode)
        } else {
            (&self.two_step_code, SecondFactor::Authenticator)
        };
        let code = Zeroizing::new(input.read(cx).value().to_string());
        self.library
            .update(cx, |lib, cx| lib.verify_two_step_code(&code, factor, cx));
    }

    /// "Use a backup code" / "Use authenticator app".
    fn toggle_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.use_backup = !self.use_backup;
        for input in [&self.two_step_code, &self.backup_code] {
            input.update(cx, |i, cx| i.set_value("", window, cx));
        }
        self.library.update(cx, Library::clear_account_error);
        self.focus_for(Screen::TwoStep, window, cx);
        cx.notify();
    }

    /// Whether the second step shows the backup code field.
    #[must_use]
    pub fn uses_backup_code(&self) -> bool {
        self.use_backup
    }

    /// Turn on… / New backup codes… / Turn off… (from the Security row or
    /// the command palette). Turning off is confirmed first.
    pub fn manage_two_step(
        &mut self,
        purpose: TwoStepPurpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (signed_in, on) = {
            let account = self.library.read(cx).account();
            (account.state().is_signed_in(), account.two_step_on())
        };
        if !signed_in || window.has_active_dialog(cx) {
            return;
        }
        let applies = match purpose {
            TwoStepPurpose::TurnOn => on != Some(true),
            TwoStepPurpose::NewBackupCodes | TwoStepPurpose::TurnOff => on == Some(true),
        };
        if !applies {
            let message = if on == Some(true) {
                "Two-step verification is already on."
            } else {
                "Two-step verification is off."
            };
            window.push_notification(message, cx);
            return;
        }
        self.two_step_dialog = None;
        let library = self.library.clone();
        let view = cx.entity().downgrade();
        let remember =
            std::rc::Rc::new(move |dialog: Option<Entity<TwoStepDialog>>, cx: &mut App| {
                _ = view.update(cx, |this, _| {
                    this.two_step_dialog = dialog.map(|d| d.downgrade());
                });
            });
        if purpose == TwoStepPurpose::TurnOff {
            ottero_ui::feedback::confirm_destructive(
                ottero_ui::feedback::Confirm::new(
                    "Turn off two-step verification?",
                    "Signing in on a new device will need only your password and Secret Key. Your backup codes stop working.",
                    "Turn off",
                ),
                window,
                cx,
                move |window, cx| {
                    // After the confirmation has closed: one dialog at a time.
                    let (library, remember) = (library.clone(), remember.clone());
                    window.defer(cx, move |window, cx| {
                        let dialog = open_two_step_dialog(library, purpose, window, cx);
                        remember(dialog, cx);
                    });
                },
            );
        } else {
            self.two_step_dialog =
                open_two_step_dialog(library, purpose, window, cx).map(|d| d.downgrade());
        }
    }

    /// The open two-step dialog, if any.
    #[must_use]
    pub fn two_step_dialog(&self) -> Option<Entity<TwoStepDialog>> {
        self.two_step_dialog
            .as_ref()
            .and_then(gpui_kit::WeakEntity::upgrade)
    }

    fn finish_kit(&mut self, cx: &mut Context<Self>) {
        if self.kit_saved {
            self.library.update(cx, Library::finish_emergency_kit);
        }
    }

    fn unlock_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let password = SecretString::from(self.unlock.read(cx).value().to_string());
        let task = self
            .library
            .update(cx, |lib, cx| lib.unlock_account(password, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.unlock.update(cx, |i, cx| i.set_value("", window, cx));
                        this.unlock_error = None;
                    }
                    Err(message) => this.unlock_error = Some(message.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// "View Secret Key" / the Copy Secret Key command: asks for the
    /// password first unless it was entered since Ottero started.
    pub fn show_secret_key(&mut self, copy: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.library.read(cx).account().is_unlocked() {
            self.reveal_secret_key = true;
            if copy {
                self.copy_secret_key(window, cx);
            }
            cx.notify();
            return;
        }
        let view = cx.entity().downgrade();
        open_password_prompt(
            self.library.clone(),
            "View Secret Key",
            "Enter your Ottero Cloud password to show your Secret Key.",
            window,
            cx,
            move |window, cx| {
                _ = view.update(cx, |this, cx| {
                    this.reveal_secret_key = true;
                    if copy {
                        this.copy_secret_key(window, cx);
                    }
                    cx.notify();
                });
            },
        );
    }

    fn copy_secret_key(&self, window: &mut Window, cx: &mut App) {
        if let Some(key) = self.library.read(cx).account().secret_key_text() {
            copy_field("Secret Key", key.to_string(), true, window, cx);
        }
    }

    /// Saves the kit; when signed in it asks for the password first.
    pub fn save_kit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let account = self.library.read(cx).account();
        if matches!(account.state(), AccountState::EmergencyKit { .. }) || account.is_unlocked() {
            save_kit(self.library.clone(), window, cx);
            return;
        }
        if !account.state().is_signed_in() {
            return;
        }
        let library = self.library.clone();
        open_password_prompt(
            self.library.clone(),
            "Save Emergency Kit",
            "Enter your Ottero Cloud password to save a kit with your Secret Key.",
            window,
            cx,
            move |window, cx| save_kit(library.clone(), window, cx),
        );
    }

    pub fn confirm_sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.library.read(cx).account().state().is_signed_in() {
            return;
        }
        let library = self.library.clone();
        ottero_ui::feedback::confirm_destructive(
            ottero_ui::feedback::Confirm::new(
                "Sign out of Ottero Cloud?",
                "Everything stays on this computer. Your Secret Key stays too, so signing back in only needs your password.",
                "Sign out",
            ),
            window,
            cx,
            move |window, cx| {
                let result = library.update(cx, Library::account_sign_out);
                ottero_ui::feedback::report(result, window, cx);
            },
        );
    }

    fn confirm_revoke(&self, id: String, label: String, window: &mut Window, cx: &mut App) {
        let library = self.library.clone();
        ottero_ui::feedback::confirm_destructive(
            ottero_ui::feedback::Confirm::new(
                format!("Sign out “{label}”?"),
                "That device stops syncing and needs the password to sign in again.",
                "Sign out",
            ),
            window,
            cx,
            move |window, cx| {
                let task = library.update(cx, |lib, cx| lib.revoke_account_session(&id, cx));
                window
                    .spawn(cx, async move |cx| {
                        if let Err(message) = task.await {
                            _ = cx.update(|window, cx| {
                                ottero_ui::feedback::error_toast(message, window, cx)
                            });
                        }
                    })
                    .detach();
            },
        );
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape"
            && matches!(
                self.screen(cx),
                Screen::SignUp | Screen::SignIn | Screen::Code | Screen::TwoStep
            )
            && !window.has_active_dialog(cx)
        {
            self.back(window, cx);
            cx.stop_propagation();
        }
    }
}

impl Render for AccountView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Drawn by the Settings page (`page`); this entity only owns state.
        div()
    }
}

/// Settings → Account, built from the current screen.
pub fn page(view: &Entity<AccountView>, cx: &App) -> SettingPage {
    let screen = view.read(cx).screen(cx);
    let part = |render: fn(&mut AccountView, &mut Context<AccountView>) -> AnyElement| {
        let view = view.clone();
        SettingItem::render(move |_, _, cx| {
            let content = view.update(cx, render);
            let key_view = view.clone();
            div()
                .id("account-content")
                .w_full()
                .capture_key_down(move |event, window, cx| {
                    key_view.update(cx, |v, cx| v.on_key(event, window, cx));
                })
                .child(content)
        })
        .keywords([
            "account",
            "ottero cloud",
            "cloud",
            "sign in",
            "sign up",
            "log in",
            "sync",
            "secret key",
            "emergency kit",
            "two-step verification",
            "two-factor",
            "2fa",
            "authenticator",
            "backup codes",
            "devices",
            "sessions",
        ])
    };
    let page = SettingPage::new("Account").icon(Lucide::CircleUser);
    match screen {
        Screen::Intro => page.group(SettingGroup::new().item(part(AccountView::render_intro))),
        Screen::SignUp => page.group(
            SettingGroup::new()
                .title("Create your Ottero Cloud account")
                .item(part(AccountView::render_sign_up)),
        ),
        Screen::SignIn => page.group(
            SettingGroup::new()
                .title("Sign in to Ottero Cloud")
                .item(part(AccountView::render_sign_in)),
        ),
        Screen::Code => page.group(
            SettingGroup::new()
                .title("Check your email")
                .item(part(AccountView::render_code)),
        ),
        Screen::TwoStep => page.group(
            SettingGroup::new()
                .title("Two-step verification")
                .item(part(AccountView::render_two_step)),
        ),
        Screen::EmergencyKit => page.group(
            SettingGroup::new()
                .title("Save your Emergency Kit")
                .item(part(AccountView::render_kit)),
        ),
        Screen::SignedIn => page
            .group(SettingGroup::new().item(part(AccountView::render_profile)))
            .group(
                SettingGroup::new()
                    .title("Ottero Cloud")
                    .item(part(AccountView::render_cloud)),
            )
            .group(
                SettingGroup::new()
                    .title("Security")
                    .item(part(AccountView::render_security)),
            )
            .group(
                SettingGroup::new()
                    .title("Devices")
                    .item(part(AccountView::render_devices)),
            ),
    }
}
