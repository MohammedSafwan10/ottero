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

use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString, Styled as _, Subscription,
    Task as GpuiTask, Window, div, px, rems,
};
use ottero_core::account::{
    AccountState, DeviceKind, Flow, Plan, SecretKeyPresence, active_label, check_password,
    device_label,
};
use ottero_crypto::{SecretString, Zeroizing};

use crate::account::Field;
use crate::account::client::{SecondFactor, TwoStepSetup};
use crate::library::{Library, LibraryEvent};
use crate::views::passwords::strength_meter;
use crate::views::qr::{QrMatrix, qr_tile};
use crate::views::vault_item::copy_field;

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

    // ---- Rendering -------------------------------------------------------

    fn render_intro(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let notice = self.library.read(cx).account().notice().map(str::to_owned);
        let point = |icon: Lucide, text: &'static str| {
            h_flex()
                .gap_3()
                .items_start()
                .child(
                    div()
                        .pt(px(2.))
                        .child(Icon::new(icon).small().text_color(theme.muted_foreground)),
                )
                .child(div().text_sm().child(text))
        };
        v_flex()
            .w_full()
            .gap_5()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(icon_tile(Lucide::Cloud, cx))
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Ottero Cloud"),
                            )
                            .child(div().text_sm().text_color(theme.muted_foreground).child(
                                "One account for Ottero on your Mac, Windows PC and phone.",
                            )),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(point(
                        Lucide::RefreshCw,
                        "Notes, tasks, calendar and passwords on every device.",
                    ))
                    .child(point(
                        Lucide::LockKeyhole,
                        "End-to-end encrypted. Only you can read your data, not even Ottero.",
                    ))
                    .child(point(
                        Lucide::HardDrive,
                        "Optional. Ottero works fully without an account.",
                    )),
            )
            .children(notice.map(|n| div().text_sm().text_color(theme.warning).child(n)))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("account-create")
                            .primary()
                            .label("Create account")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open(Mode::SignUp, window, cx)
                            })),
                    )
                    .child(Button::new("account-sign-in").label("Sign in").on_click(
                        cx.listener(|this, _, window, cx| this.open(Mode::SignIn, window, cx)),
                    )),
            )
            .child(
                div().text_xs().text_color(theme.muted_foreground).child(
                    "Sync arrives in an upcoming update. Your account will be ready for it.",
                ),
            )
            .into_any_element()
    }

    fn render_sign_up(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let account = self.library.read(cx).account();
        let busy = account.is_busy();
        let email_error = account.error_for(Field::Email).map(str::to_owned);
        let password_error = account.error_for(Field::Password).map(str::to_owned);
        let general = account.error_for(Field::General).map(str::to_owned);
        let password = self.password.read(cx).value();
        let email = self.email.read(cx).value();
        let report = (!password.is_empty()).then(|| check_password(&password, &email));
        let meter = report
            .as_ref()
            .map(|r| strength_meter(r.strength, None, cx));
        let password_note = password_error
            .map(|e| (SharedString::from(e), true))
            .or_else(|| {
                self.signup_problem(cx)
                    .map(|p| (SharedString::from(p), false))
            })
            .or_else(|| {
                report
                    .and_then(|r| r.hint)
                    .map(|h| (SharedString::from(h), false))
            });
        v_flex()
            .w_full()
            .gap_4()
            .child(field("Email", Input::new(&self.email).disabled(busy).into_any_element(), email_error.map(|e| (e.into(), true)), cx))
            .child(
                v_flex()
                    .gap_1p5()
                    .child(label("Password", cx))
                    .child(Input::new(&self.password).mask_toggle().disabled(busy))
                    .children(meter)
                    .children(password_note.map(|(text, error)| note(text, error, cx))),
            )
            .child(field("Confirm password", Input::new(&self.confirm).mask_toggle().disabled(busy).into_any_element(), None, cx))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Your password also encrypts your data. Ottero can't reset it: if you forget it, only your Emergency Kit gets you back in."),
            )
            .children(general.map(|e| note(e.into(), true, cx)))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("account-sign-up-submit")
                            .primary()
                            .label(if busy { "Creating account…" } else { "Create account" })
                            .loading(busy)
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.create_account(cx))),
                    )
                    .child(
                        Button::new("account-sign-up-back")
                            .ghost()
                            .label("Back")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("account-to-sign-in")
                            .ghost()
                            .label("I have an account")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.open(Mode::SignIn, window, cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_sign_in(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let account = self.library.read(cx).account();
        let busy = account.is_busy();
        let email_error = account.error_for(Field::Email).map(str::to_owned);
        let password_error = account.error_for(Field::Password).map(str::to_owned);
        let key_error = account.error_for(Field::SecretKey).map(str::to_owned);
        let general = account.error_for(Field::General).map(str::to_owned);
        let needs_key = self.needs_secret_key(cx);
        v_flex()
            .w_full()
            .gap_4()
            .child(field("Email", Input::new(&self.email).disabled(busy).into_any_element(), email_error.map(|e| (e.into(), true)), cx))
            .child(field(
                "Password",
                Input::new(&self.password).mask_toggle().disabled(busy).into_any_element(),
                password_error.map(|e| (e.into(), true)),
                cx,
            ))
            .when(needs_key, |this| {
                this.child(field(
                    "Secret Key",
                    Input::new(&self.secret_key).disabled(busy).into_any_element(),
                    Some(key_error.map_or_else(
                        || ("It's in your Emergency Kit. A signed-in device also shows it in Settings → Account.".into(), false),
                        |e| (e.into(), true),
                    )),
                    cx,
                ))
            })
            .children(general.map(|e| note(e.into(), true, cx)))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("account-sign-in-submit")
                            .primary()
                            .label(if busy { "Signing in…" } else { "Sign in" })
                            .loading(busy)
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.sign_in(window, cx))),
                    )
                    .child(
                        Button::new("account-sign-in-back")
                            .ghost()
                            .label("Back")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("account-to-sign-up")
                            .ghost()
                            .label("Create account")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.open(Mode::SignUp, window, cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_code(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let account = self.library.read(cx).account();
        let email = account.state().email().unwrap_or_default().to_owned();
        let busy = account.code_busy();
        let wait = account.resend_wait();
        let code_error = account.error_for(Field::Code).map(str::to_owned);
        let general = account.error_for(Field::General).map(str::to_owned);
        v_flex()
            .w_full()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_1()
                            .child("Enter the 6-digit code sent to")
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(email.clone()))
                            .child("It expires in 10 minutes."),
                    ),
            )
            .child(
                v_flex()
                    .gap_1p5()
                    .child(label("Code", cx))
                    .child(div().w(rems(12.)).child(Input::new(&self.code).large().disabled(busy)))
                    .children(code_error.map(|e| note(e.into(), true, cx))),
            )
            .children(general.map(|e| note(e.into(), true, cx)))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("account-verify")
                            .primary()
                            .label(if busy { "Verifying…" } else { "Verify" })
                            .loading(busy)
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.verify(cx))),
                    )
                    .child(
                        Button::new("account-resend")
                            .ghost()
                            .label(match wait {
                                Some(w) => format!("Resend in {} s", w.as_secs() + 1),
                                None => "Resend code".to_owned(),
                            })
                            .disabled(busy || wait.is_some())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.library.update(cx, Library::resend_account_code);
                            })),
                    )
                    .child(
                        Button::new("account-change-email")
                            .ghost()
                            .label("Change email")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.change_email(window, cx))),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("No email? Check your spam folder. If this email already has an Ottero account, no code is sent: sign in instead."),
            )
            .into_any_element()
    }

    fn render_two_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let account = self.library.read(cx).account();
        let email = account.state().email().unwrap_or_default().to_owned();
        let busy = account.code_busy();
        let code_error = account.error_for(Field::Code).map(str::to_owned);
        let general = account.error_for(Field::General).map(str::to_owned);
        let backup = self.use_backup;
        let (prompt, field_label, input) = if backup {
            (
                "Enter one of the backup codes you saved when you turned on two-step verification. Each code works once.",
                "Backup code",
                &self.backup_code,
            )
        } else {
            (
                "Enter the 6-digit code from your authenticator app.",
                "Code",
                &self.two_step_code,
            )
        };
        v_flex()
            .w_full()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .text_sm()
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_1()
                            .child("Signing in as")
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(email)),
                    )
                    .child(div().text_color(theme.muted_foreground).child(prompt)),
            )
            .child(
                v_flex()
                    .gap_1p5()
                    .child(label(field_label, cx))
                    .child(
                        div()
                            .w(rems(if backup { 14. } else { 12. }))
                            .child(Input::new(input).large().disabled(busy)),
                    )
                    .children(code_error.map(|e| note(e.into(), true, cx))),
            )
            .children(general.map(|e| note(e.into(), true, cx)))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("account-two-step-verify")
                            .primary()
                            .label(if busy { "Verifying…" } else { "Verify" })
                            .loading(busy)
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.verify_two_step(cx))),
                    )
                    .child(
                        Button::new("account-two-step-back")
                            .ghost()
                            .label("Back")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("account-two-step-switch")
                            .ghost()
                            .label(if backup {
                                "Use authenticator app"
                            } else {
                                "Use a backup code"
                            })
                            .disabled(busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_backup(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_kit(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let account = self.library.read(cx).account();
        let email = account.state().email().unwrap_or_default().to_owned();
        let secret = account
            .secret_key_text()
            .map(|k| k.to_string())
            .unwrap_or_default();
        let recovery = account
            .recovery_code_text()
            .map(|k| k.to_string())
            .unwrap_or_default();
        let busy = account.kit_busy();
        let general = account.error_for(Field::General).map(str::to_owned);
        let saved = self.kit_saved;
        v_flex()
            .w_full()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Signing in on a new device needs your password and this Secret Key. If you forget your password, the recovery code is the only way back. Ottero keeps no copy of either."),
            )
            .child(value_row("account-kit-email", "Email", email, None, cx))
            .child(value_row("account-kit-secret", "Secret Key", secret.clone(), Some("Secret Key"), cx))
            .child(value_row("account-kit-recovery", "Recovery code", recovery, Some("Recovery code"), cx))
            .child(
                h_flex().child(
                    Button::new("account-kit-save")
                        .icon(Lucide::Download)
                        .label("Save Emergency Kit…")
                        .on_click(cx.listener(|this, _, window, cx| this.save_kit(window, cx))),
                ),
            )
            .child(
                Checkbox::new("account-kit-saved")
                    .checked(saved)
                    .label("I saved my Emergency Kit")
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.kit_saved = *checked;
                        cx.notify();
                    })),
            )
            .children(general.map(|e| note(e.into(), true, cx)))
            .child(
                h_flex().child(
                    Button::new("account-kit-continue")
                        .primary()
                        .label(if busy { "Finishing…" } else { "Continue" })
                        .loading(busy)
                        .disabled(!saved || busy)
                        .on_click(cx.listener(|this, _, _, cx| this.finish_kit(cx))),
                ),
            )
            .into_any_element()
    }

    fn render_profile(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let account = self.library.read(cx).account();
        let (email, plan) = match account.state() {
            AccountState::SignedIn { email, plan } => (email.clone(), *plan),
            _ => (String::new(), Plan::Free),
        };
        let notice = account.notice().map(str::to_owned);
        let initial = email
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default();
        v_flex()
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .size(rems(2.5))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(theme.muted)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(theme.foreground)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(initial),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(email),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child(plan.label()),
                            ),
                    )
                    .child(
                        Button::new("account-sign-out")
                            .outline()
                            .small()
                            .label("Sign out…")
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.confirm_sign_out(window, cx)
                                }),
                            ),
                    ),
            )
            .children(notice.map(|n| note(n.into(), false, cx)))
            .into_any_element()
    }

    fn render_cloud(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let account = self.library.read(cx).account();
        let plan = match account.state() {
            AccountState::SignedIn { plan, .. } => *plan,
            _ => Plan::Free,
        };
        let detail = plan.detail(ottero_core::clock::now(), |t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%B %-d, %Y")
                .to_string()
        });
        let lib = self.library.read(cx);
        let pending = lib.pending_sync_count();
        let sync_state = lib.sync_state().ok();
        let (sync_value, sync_detail): (SharedString, SharedString) =
            if sync_state.as_ref().is_some_and(|s| s.needs_full_resync) {
                (
                    "Resync needed".into(),
                    "Your cursor expired; local data will re-upload on the next sync.".into(),
                )
            } else if pending > 0 {
                let word = if pending == 1 { "change" } else { "changes" };
                (
                    format!("{pending} pending").into(),
                    format!("{pending} encrypted {word} queued for upload.").into(),
                )
            } else if sync_state
                .as_ref()
                .is_some_and(|s| s.last_synced_at.is_some())
            {
                (
                    "Up to date".into(),
                    "All encrypted changes are synced with Ottero Cloud.".into(),
                )
            } else {
                (
                    "Ready".into(),
                    "Encrypted changes queue locally and sync when connected.".into(),
                )
            };

        v_flex()
            .w_full()
            .gap_3()
            .child(info_row("Plan", plan.label().into(), detail.into(), cx))
            .child(divider(cx))
            .child(info_row("Sync", sync_value, sync_detail, cx))
            .into_any_element()
    }

    fn render_security(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let account = self.library.read(cx).account();
        let unlocked = account.is_unlocked();
        let busy = account.unlock_busy();
        let secret = (unlocked && self.reveal_secret_key)
            .then(|| account.secret_key_text().map(|k| k.to_string()))
            .flatten();
        v_flex()
            .w_full()
            .gap_3()
            .when(!unlocked, |this| {
                this.child(
                    v_flex()
                        .gap_2()
                        .child(row_text(
                            "Unlock Ottero Cloud",
                            "Enter your password to open your account key on this computer.",
                            cx,
                        ))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div().w(rems(16.)).child(
                                        Input::new(&self.unlock).mask_toggle().disabled(busy),
                                    ),
                                )
                                .child(
                                    Button::new("account-unlock")
                                        .label(if busy { "Unlocking…" } else { "Unlock" })
                                        .loading(busy)
                                        .disabled(busy)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.unlock_account(window, cx)
                                        })),
                                ),
                        )
                        .children(self.unlock_error.clone().map(|e| note(e, true, cx))),
                )
                .child(divider(cx))
            })
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_center()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(div().text_sm().child("Secret Key"))
                            .child(match &secret {
                                Some(key) => div()
                                    .font_family(theme.mono_font_family.clone())
                                    .text_sm()
                                    .child(key.clone()),
                                None => div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("With your password, it signs you in on a new device."),
                            }),
                    )
                    .when_some(secret.clone(), |this, key| {
                        this.child(
                            Button::new("account-secret-copy")
                                .ghost()
                                .small()
                                .icon(Lucide::Copy)
                                .tooltip("Copy Secret Key")
                                .on_click(move |_, window, cx| {
                                    copy_field("Secret Key", key.clone(), true, window, cx)
                                }),
                        )
                        .child(
                            Button::new("account-secret-hide")
                                .outline()
                                .small()
                                .label("Hide")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.reveal_secret_key = false;
                                    cx.notify();
                                })),
                        )
                    })
                    .when(secret.is_none(), |this| {
                        this.child(
                            Button::new("account-secret-view")
                                .outline()
                                .small()
                                .label("View…")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.show_secret_key(false, window, cx)
                                })),
                        )
                    }),
            )
            .child(divider(cx))
            .child(self.render_two_step_row(cx))
            .child(divider(cx))
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_center()
                    .child(row_text(
                        "Emergency Kit",
                        "A printable page with your email and Secret Key.",
                        cx,
                    ))
                    .child(
                        Button::new("account-kit-save-again")
                            .outline()
                            .small()
                            .label("Save…")
                            .on_click(cx.listener(|this, _, window, cx| this.save_kit(window, cx))),
                    ),
            )
            .into_any_element()
    }

    /// Security → Two-step verification: its state and what can change it.
    fn render_two_step_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let on = self.library.read(cx).account().two_step_on();
        let detail = match on {
            Some(true) => {
                "Signing in on a new device also needs a code from your authenticator app."
            }
            Some(false) => {
                "Also ask for a code from an authenticator app when signing in on a new device."
            }
            None => "Checking…",
        };
        let manage = |id: &'static str, text: &'static str, purpose: TwoStepPurpose| {
            Button::new(id)
                .outline()
                .small()
                .label(text)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.manage_two_step(purpose, window, cx);
                }))
        };
        h_flex()
            .w_full()
            .gap_3()
            .items_center()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().text_sm().child("Two-step verification"))
                            .when_some(on, |this, on| {
                                this.child(
                                    div()
                                        .id("account-two-step-state")
                                        .text_xs()
                                        .px_1p5()
                                        .rounded(theme.radius)
                                        .bg(theme.muted)
                                        .text_color(if on {
                                            theme.foreground
                                        } else {
                                            theme.muted_foreground
                                        })
                                        .child(if on { "On" } else { "Off" }),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(detail),
                    ),
            )
            .map(|this| match on {
                Some(true) => this
                    .child(manage(
                        "account-two-step-codes",
                        "New backup codes…",
                        TwoStepPurpose::NewBackupCodes,
                    ))
                    .child(manage(
                        "account-two-step-off",
                        "Turn off…",
                        TwoStepPurpose::TurnOff,
                    )),
                Some(false) => this.child(manage(
                    "account-two-step-on",
                    "Turn on…",
                    TwoStepPurpose::TurnOn,
                )),
                None => this,
            })
    }

    fn render_devices(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let account = self.library.read(cx).account();
        let current = account.current_token();
        let loaded = account.sessions_loaded();
        let error = account.refresh_error().map(str::to_owned);
        let busy = account.refresh_busy();
        let now = ottero_core::clock::now();
        let mut rows: Vec<(String, String, String, DeviceKind, bool)> = account
            .sessions()
            .iter()
            .map(|s| {
                let (name, kind) = device_label(s.user_agent.as_deref());
                let active = s
                    .last_active
                    .or(s.created_at)
                    .map_or_else(String::new, |t| active_label(t, now));
                let this_device = current.as_ref().is_some_and(|t| s.is_current(t));
                (s.id.clone(), name, active, kind, this_device)
            })
            .collect();
        // This device first, then the rest as the server listed them.
        rows.sort_by_key(|r| !r.4);
        let view = cx.entity().downgrade();
        let mut list = v_flex().w_full();
        if !loaded {
            list = list.child(
                h_flex()
                    .gap_2()
                    .py_1()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .map(|this| match &error {
                        Some(e) => this.child(e.clone()),
                        None => this.child(Spinner::new().small()).child("Loading devices…"),
                    })
                    .when(error.is_some(), |this| {
                        this.child(
                            Button::new("account-devices-retry")
                                .ghost()
                                .small()
                                .label("Try again")
                                .loading(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.library.update(cx, Library::refresh_account)
                                })),
                        )
                    }),
            );
        }
        for (i, (id, name, active, kind, this_device)) in rows.into_iter().enumerate() {
            let icon = match kind {
                DeviceKind::Phone => Lucide::Smartphone,
                DeviceKind::Browser => Lucide::Globe,
                DeviceKind::Computer | DeviceKind::Unknown => Lucide::Laptop,
            };
            let label_for_confirm = name.clone();
            let view = view.clone();
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("session-{id}")))
                    .w_full()
                    .gap_3()
                    .py_2()
                    .items_center()
                    .when(i > 0, |s| s.border_t_1().border_color(theme.border))
                    .child(Icon::new(icon).text_color(theme.muted_foreground))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(h_flex().gap_2().child(div().text_sm().child(name)).when(
                                this_device,
                                |this| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .px_1p5()
                                            .rounded(theme.radius)
                                            .bg(theme.muted)
                                            .text_color(theme.muted_foreground)
                                            .child("This device"),
                                    )
                                },
                            ))
                            .when(!active.is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(active),
                                )
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("session-sign-out-{id}")))
                            .ghost()
                            .small()
                            .label(if this_device {
                                "Sign out of this device…"
                            } else {
                                "Sign out…"
                            })
                            .on_click(move |_, window, cx| {
                                _ = view.update(cx, |this, cx| {
                                    if this_device {
                                        this.confirm_sign_out(window, cx);
                                    } else {
                                        this.confirm_revoke(
                                            id.clone(),
                                            label_for_confirm.clone(),
                                            window,
                                            cx,
                                        );
                                    }
                                });
                            }),
                    ),
            );
        }
        list.into_any_element()
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

fn icon_tile(icon: Lucide, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .size(rems(2.5))
        .flex_shrink_0()
        .rounded(theme.radius_lg)
        .bg(theme.muted)
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(icon).text_color(theme.foreground))
}

fn label(text: &'static str, cx: &App) -> impl IntoElement {
    let _ = cx;
    div().text_sm().font_weight(FontWeight::MEDIUM).child(text)
}

fn note(text: SharedString, error: bool, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .text_xs()
        .text_color(if error {
            theme.danger
        } else {
            theme.muted_foreground
        })
        .child(text)
        .into_any_element()
}

fn field(
    name: &'static str,
    input: AnyElement,
    message: Option<(SharedString, bool)>,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .gap_1p5()
        .child(label(name, cx))
        .child(input)
        .children(message.map(|(text, error)| note(text, error, cx)))
}

fn divider(cx: &App) -> impl IntoElement {
    div().w_full().h(px(1.)).bg(cx.theme().border)
}

fn row_text(title: &'static str, description: &'static str, cx: &App) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap_0p5()
        .child(div().text_sm().child(title))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(description),
        )
}

fn info_row(
    title: &'static str,
    value: SharedString,
    detail: SharedString,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .w_full()
        .gap_3()
        .items_start()
        .child(
            div()
                .w(rems(6.))
                .flex_shrink_0()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(title),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(value))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(detail),
                ),
        )
}

/// A label, a monospace value and (for secrets) a Copy button.
fn value_row(
    id: &'static str,
    title: &'static str,
    value: String,
    copy_as: Option<&'static str>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let copy_value = value.clone();
    v_flex().gap_1p5().child(label(title, cx)).child(
        h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .px_3()
            .py_2()
            .rounded(theme.radius)
            .bg(theme.muted)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(copy_as.is_some(), |d| {
                        d.font_family(theme.mono_font_family.clone())
                    })
                    .text_sm()
                    .child(value),
            )
            .when_some(copy_as, |this, name| {
                this.child(
                    Button::new(id)
                        .ghost()
                        .xsmall()
                        .icon(Lucide::Copy)
                        .tooltip(format!("Copy {name}"))
                        .on_click(move |_, window, cx| {
                            copy_field(name, copy_value.clone(), true, window, cx)
                        }),
                )
            }),
    )
}

/// Writes the Emergency Kit where the user picks, then opens it in the
/// browser to print. The file is readable only by the user.
pub fn save_kit(library: Entity<Library>, window: &mut Window, cx: &mut App) {
    let created = ottero_core::clock::today()
        .strftime("%B %-d, %Y")
        .to_string();
    let Some(html) = library.read(cx).emergency_kit_html(&created) else {
        ottero_ui::feedback::error_toast(
            "Enter your password first to include your Secret Key.",
            window,
            cx,
        );
        return;
    };
    let dir = directories::UserDirs::new()
        .and_then(|d| d.document_dir().map(std::path::Path::to_path_buf))
        .unwrap_or_else(std::env::temp_dir);
    let path = cx.prompt_for_new_path(&dir, Some("Ottero Emergency Kit.html"));
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            let written = crate::private_file::write_private(&path, html.as_bytes());
            _ = cx.update(|window, cx| match written {
                Ok(()) => {
                    cx.open_with_system(&path);
                    window.push_notification(
                        "Emergency Kit saved. Print it or save it as a PDF, then keep it off this computer.",
                        cx,
                    );
                }
                Err(_) => ottero_ui::feedback::error_toast("Couldn't save the Emergency Kit.", window, cx),
            });
        })
        .detach();
}

/// Asks for the account password, then runs `then`. Escape cancels.
fn open_password_prompt(
    library: Entity<Library>,
    title: &'static str,
    description: &'static str,
    window: &mut Window,
    cx: &mut App,
    then: impl Fn(&mut Window, &mut App) + 'static,
) {
    if window.has_active_dialog(cx) {
        return;
    }
    let prompt =
        cx.new(|cx| PasswordPrompt::new(library, description, std::rc::Rc::new(then), window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title(title).w(px(420.)).child(prompt.clone())
    });
}

/// What runs after the password was accepted.
type AfterUnlock = std::rc::Rc<dyn Fn(&mut Window, &mut App)>;

struct PasswordPrompt {
    library: Entity<Library>,
    description: &'static str,
    input: Entity<InputState>,
    error: Option<SharedString>,
    busy: bool,
    then: AfterUnlock,
    _subscription: Subscription,
}

impl PasswordPrompt {
    fn new(
        library: Entity<Library>,
        description: &'static str,
        then: AfterUnlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Password")
        });
        let subscription =
            cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                InputEvent::Change => {
                    this.error = None;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.submit(window, cx),
                _ => {}
            });
        input.update(cx, |i, cx| i.focus(window, cx));
        Self {
            library,
            description,
            input,
            error: None,
            busy: false,
            then,
            _subscription: subscription,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        let password = SecretString::from(self.input.read(cx).value().to_string());
        let task = self
            .library
            .update(cx, |lib, cx| lib.unlock_account(password, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        window.close_dialog(cx);
                        (this.then)(window, cx);
                    }
                    Err(message) => this.error = Some(message.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for PasswordPrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        v_flex()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(self.description),
            )
            .child(Input::new(&self.input).mask_toggle().disabled(self.busy))
            .children(self.error.clone().map(|e| note(e, true, cx)))
            .child(
                h_flex().justify_end().child(
                    Button::new("account-prompt-submit")
                        .primary()
                        .label(if self.busy { "Checking…" } else { "Continue" })
                        .loading(self.busy)
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                ),
            )
    }
}

/// Which change [`TwoStepDialog`] makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwoStepPurpose {
    TurnOn,
    NewBackupCodes,
    TurnOff,
}

/// Where [`TwoStepDialog`] is.
///
/// ```text
/// TurnOn:          Password ──▶ Scan ──code accepted──▶ BackupCodes ──Done──▶ closed
/// NewBackupCodes:  Password ──▶ BackupCodes ──Done──▶ closed
/// TurnOff:         (confirmed first) Password ──▶ closed
/// ```
///
/// `BackupCodes` can only be left with Done, after "I saved my backup
/// codes": the codes are never shown again. While a request runs the dialog
/// can't be dismissed either, so an answer that turns two-step on always
/// reaches the backup codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwoStepStep {
    Password,
    Scan,
    BackupCodes,
}

/// Turning two-step verification on (password → QR code and setup key →
/// code from the app → backup codes), new backup codes, or turning it off.
pub struct TwoStepDialog {
    library: Entity<Library>,
    purpose: TwoStepPurpose,
    step: TwoStepStep,
    focus: gpui_kit::FocusHandle,
    password: Entity<InputState>,
    code: Entity<InputState>,
    setup: Option<std::rc::Rc<TwoStepSetup>>,
    qr: Option<std::rc::Rc<QrMatrix>>,
    codes: Vec<Zeroizing<String>>,
    saved: bool,
    busy: bool,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

/// Opens [`TwoStepDialog`]. The dialog locks (no Escape, no close button)
/// while a request runs and while the backup codes are shown.
pub fn open_two_step_dialog(
    library: Entity<Library>,
    purpose: TwoStepPurpose,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<TwoStepDialog>> {
    if window.has_active_dialog(cx) {
        return None;
    }
    let view = cx.new(|cx| TwoStepDialog::new(library, purpose, window, cx));
    let opened = view.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let this = view.read(cx);
        let locked = this.busy || this.step == TwoStepStep::BackupCodes;
        dialog
            .title(this.title())
            .w(px(480.))
            .keyboard(!locked)
            .overlay_closable(!locked)
            .close_button(!locked)
            // Enter belongs to the step: the fields submit themselves, and on
            // the backup codes it means Done (once saved). It never just closes.
            .on_ok({
                let view = view.clone();
                move |_, window, cx| {
                    view.update(cx, |this, cx| this.confirm(window, cx));
                    false
                }
            })
            .child(view.clone())
    });
    // Opening focuses the dialog itself; the password field goes first.
    opened.update(cx, |this, cx| {
        this.password.update(cx, |i, cx| i.focus(window, cx))
    });
    Some(opened)
}

impl TwoStepDialog {
    fn new(
        library: Entity<Library>,
        purpose: TwoStepPurpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Password")
        });
        let code = cx.new(|cx| InputState::new(window, cx).placeholder("123 456"));
        let subscriptions = vec![
            cx.subscribe_in(
                &password,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::Change => {
                        this.error = None;
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => this.submit_password(window, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(&code, window, |this, _, event, window, cx| match event {
                InputEvent::Change => this.code_changed(window, cx),
                InputEvent::PressEnter { .. } => this.submit_code(window, cx),
                _ => {}
            }),
        ];
        Self {
            library,
            purpose,
            step: TwoStepStep::Password,
            focus: cx.focus_handle(),
            password,
            code,
            setup: None,
            qr: None,
            codes: Vec::new(),
            saved: false,
            busy: false,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    #[must_use]
    pub fn step(&self) -> TwoStepStep {
        self.step
    }

    #[must_use]
    pub fn purpose(&self) -> TwoStepPurpose {
        self.purpose
    }

    /// The `otpauth://` link drawn as the QR code (Scan step).
    #[must_use]
    pub fn setup_uri(&self) -> Option<&str> {
        self.setup.as_deref().map(TwoStepSetup::uri)
    }

    /// The QR code shown (Scan step).
    #[must_use]
    pub fn qr(&self) -> Option<&QrMatrix> {
        self.qr.as_deref()
    }

    /// The backup codes shown (BackupCodes step).
    #[must_use]
    pub fn backup_codes(&self) -> Vec<String> {
        self.codes.iter().map(|c| c.to_string()).collect()
    }

    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn title(&self) -> &'static str {
        match (self.purpose, self.step) {
            (_, TwoStepStep::BackupCodes) => "Save your backup codes",
            (TwoStepPurpose::TurnOn, _) => "Turn on two-step verification",
            (TwoStepPurpose::NewBackupCodes, _) => "New backup codes",
            (TwoStepPurpose::TurnOff, _) => "Turn off two-step verification",
        }
    }

    fn submit_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.step != TwoStepStep::Password {
            return;
        }
        let password = SecretString::from(self.password.read(cx).value().to_string());
        self.busy = true;
        self.error = None;
        cx.notify();
        match self.purpose {
            TwoStepPurpose::TurnOn => {
                let task = self
                    .library
                    .update(cx, |lib, cx| lib.start_two_step_setup(password, cx));
                cx.spawn_in(window, async move |this, cx| {
                    let result = task.await;
                    _ = this.update_in(cx, |this, window, cx| {
                        this.busy = false;
                        match result {
                            Ok(setup) => {
                                this.qr = QrMatrix::encode(setup.uri()).map(std::rc::Rc::new);
                                this.codes = setup.backup_codes().to_vec();
                                this.setup = Some(std::rc::Rc::new(setup));
                                this.step = TwoStepStep::Scan;
                                this.password
                                    .update(cx, |i, cx| i.set_value("", window, cx));
                                this.code.update(cx, |i, cx| i.focus(window, cx));
                            }
                            Err(message) => this.error = Some(message.into()),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            TwoStepPurpose::NewBackupCodes => {
                let task = self
                    .library
                    .update(cx, |lib, cx| lib.new_backup_codes(password, cx));
                cx.spawn_in(window, async move |this, cx| {
                    let result = task.await;
                    _ = this.update_in(cx, |this, window, cx| {
                        this.busy = false;
                        match result {
                            Ok(codes) => this.show_codes(codes, window, cx),
                            Err(message) => this.error = Some(message.into()),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            TwoStepPurpose::TurnOff => {
                let task = self
                    .library
                    .update(cx, |lib, cx| lib.turn_off_two_step(password, cx));
                cx.spawn_in(window, async move |this, cx| {
                    let result = task.await;
                    _ = this.update_in(cx, |this, window, cx| {
                        this.busy = false;
                        match result {
                            Ok(()) => window.close_dialog(cx),
                            Err(message) => this.error = Some(message.into()),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        }
    }

    fn show_codes(
        &mut self,
        codes: Vec<Zeroizing<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.codes = codes;
        self.step = TwoStepStep::BackupCodes;
        self.saved = false;
        self.password
            .update(cx, |i, cx| i.set_value("", window, cx));
        // Keep keyboard focus inside the dialog once the code field is gone.
        self.focus.focus(window, cx);
    }

    /// Digits only, "123 456"; the sixth digit submits.
    fn code_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.code.read(cx).value();
        let digits = ottero_core::account::code_digits(&value);
        let shown = ottero_core::account::group_code(&digits);
        if shown != value.as_ref() {
            self.code.update(cx, |i, cx| i.set_value(shown, window, cx));
        }
        self.error = None;
        if digits.len() == 6 {
            self.submit_code(window, cx);
        }
        cx.notify();
    }

    fn submit_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.step != TwoStepStep::Scan {
            return;
        }
        let code = Zeroizing::new(self.code.read(cx).value().to_string());
        self.busy = true;
        self.error = None;
        cx.notify();
        let task = self
            .library
            .update(cx, |lib, cx| lib.confirm_two_step_setup(&code, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        let codes = std::mem::take(&mut this.codes);
                        this.show_codes(codes, window, cx);
                    }
                    Err(message) => {
                        this.error = Some(message.into());
                        this.code.update(cx, |i, cx| i.focus(window, cx));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn copy_codes(&self, window: &mut Window, cx: &mut App) {
        let all = self
            .codes
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        copy_field("Backup codes", all, true, window, cx);
    }

    /// Enter: the current step's action (each one ignores a repeat while busy).
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.step {
            TwoStepStep::Password => self.submit_password(window, cx),
            TwoStepStep::Scan => self.submit_code(window, cx),
            TwoStepStep::BackupCodes => self.done(window, cx),
        }
    }

    fn done(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.step == TwoStepStep::BackupCodes && self.saved {
            self.codes.clear();
            window.close_dialog(cx);
        }
    }

    fn render_password(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (description, button, busy_label) = match self.purpose {
            TwoStepPurpose::TurnOn => (
                "Enter your Ottero Cloud password to set up an authenticator app.",
                "Continue",
                "Checking…",
            ),
            TwoStepPurpose::NewBackupCodes => (
                "Enter your Ottero Cloud password. Your current backup codes will stop working.",
                "Continue",
                "Checking…",
            ),
            TwoStepPurpose::TurnOff => (
                "Enter your Ottero Cloud password to turn off two-step verification.",
                "Turn off",
                "Turning off…",
            ),
        };
        let submit = Button::new("two-step-password-submit")
            .label(if self.busy { busy_label } else { button })
            .loading(self.busy)
            .disabled(self.busy)
            .on_click(cx.listener(|this, _, window, cx| this.submit_password(window, cx)));
        let submit = if self.purpose == TwoStepPurpose::TurnOff {
            submit.danger()
        } else {
            submit.primary()
        };
        v_flex()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(description),
            )
            .child(Input::new(&self.password).mask_toggle().disabled(self.busy))
            .children(self.error.clone().map(|e| note(e, true, cx)))
            .child(h_flex().justify_end().child(submit))
            .into_any_element()
    }

    fn render_scan(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let key = self
            .setup
            .as_ref()
            .map(|s| s.setup_key().to_owned())
            .unwrap_or_default();
        let grouped = ottero_core::account::group_setup_key(&key);
        let code_tile = match &self.qr {
            Some(qr) => qr_tile(qr.clone(), rems(11.), cx).into_any_element(),
            None => div()
                .size(rems(11.))
                .flex_shrink_0()
                .rounded(theme.radius)
                .bg(theme.muted)
                .p_3()
                .flex()
                .items_center()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("The QR code couldn't be drawn. Use the setup key.")
                .into_any_element(),
        };
        v_flex()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Scan this QR code with an authenticator app, such as 1Password, Google Authenticator or Microsoft Authenticator."),
            )
            .child(
                h_flex()
                    .gap_4()
                    .items_start()
                    .child(code_tile)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1p5()
                            .child(label("Setup key", cx))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("Can't scan? Type this key into the app instead."),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_start()
                                    .px_3()
                                    .py_2()
                                    .rounded(theme.radius)
                                    .bg(theme.muted)
                                    .child(
                                        div()
                                            .id("two-step-setup-key")
                                            .flex_1()
                                            .min_w_0()
                                            .font_family(theme.mono_font_family.clone())
                                            .text_sm()
                                            .child(grouped),
                                    )
                                    .child(
                                        Button::new("two-step-copy-key")
                                            .ghost()
                                            .xsmall()
                                            .icon(Lucide::Copy)
                                            .tooltip("Copy setup key")
                                            .on_click(move |_, window, cx| {
                                                copy_field("Setup key", key.clone(), true, window, cx);
                                            }),
                                    ),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_1p5()
                    .child(label("Code from the app", cx))
                    .child(
                        div()
                            .w(rems(12.))
                            .child(Input::new(&self.code).large().disabled(self.busy)),
                    )
                    .children(self.error.clone().map(|e| note(e, true, cx))),
            )
            .child(
                h_flex().justify_end().child(
                    Button::new("two-step-confirm")
                        .primary()
                        .label(if self.busy { "Turning on…" } else { "Turn on" })
                        .loading(self.busy)
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.submit_code(window, cx))),
                ),
            )
            .into_any_element()
    }

    fn render_codes(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let lead = match self.purpose {
            TwoStepPurpose::TurnOn => "Two-step verification is on.",
            _ => "Your old backup codes no longer work.",
        };
        let half = self.codes.len().div_ceil(2);
        let column = |codes: &[Zeroizing<String>]| {
            v_flex()
                .gap_1()
                .children(codes.iter().map(|c| div().child(c.to_string())))
        };
        let saved = self.saved;
        v_flex()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .text_sm()
                    .child(div().font_weight(FontWeight::MEDIUM).child(lead))
                    .child(div().text_color(theme.muted_foreground).child(
                        "Each backup code signs you in once if you can't use your authenticator app. Keep them somewhere safe, like your password manager. They won't be shown again.",
                    )),
            )
            .child(
                h_flex()
                    .id("two-step-backup-codes")
                    .w_full()
                    .justify_center()
                    .gap_8()
                    .px_4()
                    .py_3()
                    .rounded(theme.radius)
                    .bg(theme.muted)
                    .font_family(theme.mono_font_family.clone())
                    .text_sm()
                    .child(column(&self.codes[..half]))
                    .child(column(&self.codes[half..])),
            )
            .child(
                h_flex().child(
                    Button::new("two-step-copy-codes")
                        .outline()
                        .small()
                        .icon(Lucide::Copy)
                        .label("Copy all")
                        .on_click(cx.listener(|this, _, window, cx| this.copy_codes(window, cx))),
                ),
            )
            .child(
                Checkbox::new("two-step-codes-saved")
                    .checked(saved)
                    .label("I saved my backup codes")
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.saved = *checked;
                        cx.notify();
                    })),
            )
            .child(
                h_flex().justify_end().child(
                    Button::new("two-step-done")
                        .primary()
                        .label("Done")
                        .disabled(!saved)
                        .on_click(cx.listener(|this, _, window, cx| this.done(window, cx))),
                ),
            )
            .into_any_element()
    }
}

impl gpui_kit::Focusable for TwoStepDialog {
    fn focus_handle(&self, _: &App) -> gpui_kit::FocusHandle {
        self.focus.clone()
    }
}

impl Render for TwoStepDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.step {
            TwoStepStep::Password => self.render_password(cx),
            TwoStepStep::Scan => self.render_scan(cx),
            TwoStepStep::BackupCodes => self.render_codes(cx),
        };
        div()
            .id("two-step-dialog")
            .track_focus(&self.focus)
            .child(content)
    }
}
