//! Privacy & data settings page: vault security, backups, archives, and local storage.

use std::path::PathBuf;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{App, Entity, IntoElement, ParentElement as _, SharedString, Styled as _, div};
use ottero_ui::components::segmented;

pub(super) fn privacy_page(
    data_dir: PathBuf,
    library: Entity<crate::library::Library>,
) -> SettingPage {
    let shown = SharedString::from(data_dir.display().to_string());
    let location = SettingItem::new(
        "Data folder",
        SettingField::render(move |_, _, _| {
            let dir = data_dir.clone();
            Button::new("reveal-data")
                .outline()
                .small()
                .label(if cfg!(target_os = "macos") {
                    "Show in Finder"
                } else {
                    "Open folder"
                })
                .on_click(move |_, _, cx| cx.reveal_path(&dir))
        }),
    )
    .description(shown)
    .keywords(["storage", "database", "backup", "location"]);

    SettingPage::new("Privacy & data")
        .icon(Lucide::ShieldCheck)
        .groups(password_groups(library))
        .group(SettingGroup::new().title("Storage").item(location).footer(
            |_, _| "Local-first SQLite storage. Optional encrypted sync is managed under Account.",
        ))
}

fn password_groups(library: Entity<crate::library::Library>) -> Vec<SettingGroup> {
    use ottero_core::vault::lock::{AutoLock, ClipboardClear};
    let auto: &'static [(AutoLock, &'static str)] = &[
        (AutoLock::OneMinute, "1 min"),
        (AutoLock::FiveMinutes, "5 min"),
        (AutoLock::FifteenMinutes, "15 min"),
        (AutoLock::OneHour, "1 hour"),
        (AutoLock::Never, "Never"),
    ];
    let clear: &'static [(ClipboardClear, &'static str)] = &[
        (ClipboardClear::FifteenSeconds, "15 s"),
        (ClipboardClear::ThirtySeconds, "30 s"),
        (ClipboardClear::NinetySeconds, "90 s"),
    ];
    let auto_lock = SettingItem::new(
        "Lock after",
        SettingField::render(move |_, _, cx| {
            segmented(
                "set-auto-lock",
                auto,
                crate::vault_prefs::get(cx).auto_lock,
                |v, _, cx| crate::vault_prefs::update(cx, |p| p.auto_lock = v),
            )
        }),
    )
    .description("Lock the vault when you haven't used Ottero for this long.")
    .keywords(["vault", "auto-lock", "timeout", "security", "passwords"]);
    let hide = SettingItem::new(
        "Lock when Ottero is in the background",
        SettingField::switch(
            |cx| crate::vault_prefs::get(cx).lock_on_hide,
            |on, cx| crate::vault_prefs::update(cx, |p| p.lock_on_hide = on),
        ),
    )
    .description("Locks as soon as you switch to another app.")
    .keywords(["vault", "lock", "background", "security"]);
    let clipboard = SettingItem::new(
        "Clear copied passwords after",
        SettingField::render(move |_, _, cx| {
            segmented(
                "set-clipboard",
                clear,
                crate::vault_prefs::get(cx).clipboard,
                |v, _, cx| crate::vault_prefs::update(cx, |p| p.clipboard = v),
            )
        }),
    )
    .description("Only if the clipboard still holds what Ottero copied.")
    .keywords(["clipboard", "copy", "vault", "security"]);
    let linked = {
        let library = library.clone();
        move |cx: &App| {
            let lib = library.read(cx);
            (lib.vault_opener() == Some(ottero_core::vault::lock::Opener::Account))
                .then(|| lib.vault_linked_email().unwrap_or_default().to_owned())
        }
    };
    let signed_in = {
        let library = library.clone();
        move |cx: &App| library.read(cx).account().signed_in_email().is_some()
    };
    let master = SettingItem::new(
        "Unlock with",
        SettingField::render(move |_, _, cx| match linked(cx) {
            Some(email) => div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(format!("Ottero account · {email}"))
                .into_any_element(),
            None => h_flex()
                .gap_2()
                .when(signed_in(cx), |s| {
                    s.child(
                        Button::new("set-use-account")
                            .small()
                            .ghost()
                            .label("Use Ottero account…")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(
                                    Box::new(crate::actions::UseAccountForPasswords),
                                    cx,
                                )
                            }),
                    )
                })
                .child(
                    Button::new("set-change-master")
                        .small()
                        .label("Change master password…")
                        .on_click(|_, window, cx| {
                            window
                                .dispatch_action(Box::new(crate::actions::ChangeMasterPassword), cx)
                        }),
                )
                .into_any_element(),
        }),
    )
    .description("A master password of its own, or your Ottero account when you're signed in. Linking asks for the master password once and then retires it.")
    .keywords(["master password", "vault", "change password", "ottero account", "cloud", "link"]);
    let recovery = SettingItem::new(
        "Recovery key",
        SettingField::render(|_, _, _| {
            Button::new("set-recovery")
                .small()
                .label("Create new key…")
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(crate::actions::CreateRecoveryKey), cx)
                })
        }),
    )
    .description(
        "Opens your vault if you forget your password or can't sign in. A new key replaces the old one.",
    )
    .keywords(["recovery", "emergency kit", "forgot password", "vault"]);
    let archives = SettingItem::new(
        "Archived vaults",
        SettingField::render(|_, _, _| {
            Button::new("set-archives")
                .small()
                .label("Show…")
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(crate::actions::ShowVaultArchives), cx)
                })
        }),
    )
    .description("Old vaults kept when you started a new one. Open one with its old password to get its items back.")
    .keywords(["archive", "old vault", "restore", "forgot password"]);
    let backup = SettingItem::new(
        "Encrypted backup",
        SettingField::render(|_, _, _| {
            h_flex()
                .gap_2()
                .child(
                    Button::new("set-vault-restore")
                        .small()
                        .ghost()
                        .label("Restore…")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(crate::actions::RestoreVaultBackup), cx)
                        }),
                )
                .child(
                    Button::new("set-vault-export")
                        .small()
                        .label("Export…")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(crate::actions::ExportVaultBackup), cx)
                        }),
                )
        }),
    )
    .description("A copy of your vault that stays encrypted. It opens with the master password you have when you save it, or with your Ottero account if Passwords uses your account.")
    .keywords(["backup", "export", "restore", "vault", "copy"]);
    vec![
        SettingGroup::new()
            .title("Password security")
            .items(vec![auto_lock, hide, clipboard]),
        SettingGroup::new()
            .title("Vault & backups")
            .items(vec![master, recovery, backup, archives]),
    ]
}
