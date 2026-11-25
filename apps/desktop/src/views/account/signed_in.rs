//! Signed-in Account settings panels: profile, Cloud & Sync, security, devices.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, div, rems,
};
use ottero_core::account::{AccountState, DeviceKind, Plan, active_label, device_label};

use super::widgets::{divider, info_row, note, row_text};
use super::{AccountView, TwoStepPurpose};
use crate::library::Library;
use crate::views::vault_item::copy_field;

impl AccountView {
    pub(super) fn render_profile(&mut self, cx: &mut Context<Self>) -> AnyElement {
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

    pub(super) fn render_cloud(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
        let unlocked = account.is_unlocked();
        let syncing = lib.sync_in_flight();
        let sync_error = lib.sync_last_error().map(str::to_owned);
        let pending = lib.pending_sync_count();
        let sync_state = lib.sync_state().ok();
        let (sync_value, sync_detail): (SharedString, SharedString) = if syncing {
            (
                "Syncing…".into(),
                "Encrypting and exchanging changes with Ottero Cloud.".into(),
            )
        } else if let Some(err) = sync_error.clone() {
            ("Sync issue".into(), err.into())
        } else if sync_state.as_ref().is_some_and(|s| s.needs_full_resync) {
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

        let plan_action: Option<AnyElement> = match plan {
            Plan::Free => Some(
                Button::new("account-upgrade")
                    .small()
                    .primary()
                    .label("Upgrade to Cloud")
                    .on_click(cx.listener(|this, _, window, cx| {
                        let task = this
                            .library
                            .update(cx, |lib, cx| lib.billing_checkout_url("yearly", cx));
                        window
                            .spawn(cx, async move |cx| {
                                let res = task.await;
                                _ = cx.update(|window, cx| match res {
                                    Ok(url) => cx.open_url(&url),
                                    Err(msg) => ottero_ui::feedback::error_toast(&msg, window, cx),
                                });
                            })
                            .detach();
                    }))
                    .into_any_element(),
            ),
            Plan::Cloud {
                source: ottero_core::account::CloudSource::Paid,
                ..
            } => Some(
                Button::new("account-manage-billing")
                    .small()
                    .label("Manage billing")
                    .on_click(cx.listener(|this, _, window, cx| {
                        let task = this
                            .library
                            .update(cx, |lib, cx| lib.billing_portal_url(cx));
                        window
                            .spawn(cx, async move |cx| {
                                let res = task.await;
                                _ = cx.update(|window, cx| match res {
                                    Ok(url) => cx.open_url(&url),
                                    Err(msg) => ottero_ui::feedback::error_toast(&msg, window, cx),
                                });
                            })
                            .detach();
                    }))
                    .into_any_element(),
            ),
            _ => None,
        };

        v_flex()
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(info_row("Plan", plan.label().into(), detail.into(), cx))
                    .children(plan_action),
            )
            .child(divider(cx))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(info_row("Sync", sync_value, sync_detail, cx))
                    .child(
                        Button::new("account-sync-now")
                            .small()
                            .label(if syncing { "Syncing…" } else { "Sync now" })
                            .loading(syncing)
                            .disabled(syncing || !unlocked)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.library.update(cx, Library::sync_now);
                            })),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_security(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    div().w(rems(16.)).max_w_full().child(
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
            .flex_wrap()
            .gap_3()
            .items_center()
            .child(
                v_flex()
                    .flex_1()
                    .min_w(rems(12.))
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

    pub(super) fn render_devices(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
