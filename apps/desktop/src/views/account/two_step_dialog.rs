//! Two-step verification (TOTP + backup codes) modal dialog.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
    px, rems,
};
use ottero_crypto::{SecretString, Zeroizing};

use crate::account::client::TwoStepSetup;
use crate::library::Library;
use crate::views::qr::{QrMatrix, qr_tile};
use crate::views::vault_item::copy_field;

use super::widgets::{label, note};

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
