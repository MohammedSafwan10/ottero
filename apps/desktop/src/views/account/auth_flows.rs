//! Signed-out, sign-up, sign-in, verification, and Emergency Kit screens.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, FontWeight, IntoElement as _, ParentElement as _, SharedString,
    Styled as _, div, px, rems,
};
use ottero_core::account::check_password;

use crate::account::Field;
use crate::views::passwords::strength_meter;

use super::widgets::{field, icon_tile, label, note, value_row};
use super::{AccountView, Mode};
use crate::library::Library;

impl AccountView {
    // ---- Rendering -------------------------------------------------------

    pub(super) fn render_intro(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
                    "End-to-end encrypted sync keeps your notes, tasks, calendar, habits, and vault in sync.",
                ),
            )
            .into_any_element()
    }

    pub(super) fn render_sign_up(&mut self, cx: &mut Context<Self>) -> AnyElement {
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

    pub(super) fn render_sign_in(&mut self, cx: &mut Context<Self>) -> AnyElement {
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

    pub(super) fn render_code(&mut self, cx: &mut Context<Self>) -> AnyElement {
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

    pub(super) fn render_two_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
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

    pub(super) fn render_kit(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
}
