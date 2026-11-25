//! Settings screen, built on GPUI Kit's searchable `Settings` component.
//!
//! Pages: Account, General, Workspace, Appearance, Features, Privacy & data, Keyboard,
//! About. Feature preferences stay grouped under Features; search covers all pages.

use std::path::PathBuf;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::ThemeMode;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::setting::{
    RenderOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, InteractiveElement as _, IntoElement, Keystroke,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, TestSupportExt as _, Window, div,
};
use ottero_ui::appearance::{
    self, AppearanceSettings, ClockFormat, ContentWidth, LineSpacing, MonoFont, MotionPreference,
    ThemePreference, UiFont, WeekStart,
};
use ottero_ui::components::segmented;

use crate::actions::{
    CALENDAR_SHORTCUTS, EDITING_SHORTCUTS, HABIT_SHORTCUTS, SHORTCUTS, ShortcutGroup,
};

pub struct SettingsView {
    reorder_focus: std::collections::BTreeMap<&'static str, (FocusHandle, FocusHandle)>,
    layout: Entity<crate::ui_state::LayoutStore>,
    generation: u64,
    initial_page: usize,
    show_changelog: bool,
    changelog_version: usize,
    _layout_observer: Subscription,
    _updates_observer: Subscription,
    _account_observer: Subscription,
    account: Entity<crate::views::account::AccountView>,
    data_dir: PathBuf,
    focus: FocusHandle,
    /// Created on first render (it needs a window).
    holiday_pickers: Option<crate::views::holiday_settings::HolidayPickers>,
}

impl SettingsView {
    pub fn new(
        data_dir: PathBuf,
        layout: Entity<crate::ui_state::LayoutStore>,
        account: Entity<crate::views::account::AccountView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observer = cx.observe(&layout, |_, _, cx| cx.notify());
        // The Account page is drawn here from the account view's state.
        let account_observer = cx.observe(&account, |_, _, cx| cx.notify());
        let updates_observer = cx.observe_global::<crate::updates::Updates>(|_, cx| cx.notify());
        Self {
            reorder_focus: crate::nav::entries()
                .map(|e| (e.id, (cx.focus_handle(), cx.focus_handle())))
                .collect(),
            layout,
            generation: 0,
            initial_page: 0,
            show_changelog: false,
            changelog_version: 0,
            _layout_observer: observer,
            _updates_observer: updates_observer,
            _account_observer: account_observer,
            account,
            data_dir,
            focus: cx.focus_handle(),
            holiday_pickers: None,
        }
    }
    /// Page indexes in `render`'s list.
    const ACCOUNT_PAGE: usize = 0;
    const WORKSPACE_PAGE: usize = 2;
    const ABOUT_PAGE: usize = 7;

    #[must_use]
    pub fn account(&self) -> &Entity<crate::views::account::AccountView> {
        &self.account
    }

    /// Shows Settings → Account (and, signed out, the given form).
    pub fn open_account(
        &mut self,
        mode: Option<crate::views::account::Mode>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_changelog = false;
        self.initial_page = Self::ACCOUNT_PAGE;
        self.generation = self.generation.wrapping_add(1);
        self.account.update(cx, |a, cx| match mode {
            Some(mode) => a.open(mode, window, cx),
            None => cx.notify(),
        });
        self.library_refresh(cx);
        cx.notify();
    }

    fn library_refresh(&self, cx: &mut Context<Self>) {
        let library = self.account.read(cx).library().clone();
        library.update(cx, crate::library::Library::refresh_account);
    }

    pub fn open_workspace(&mut self, cx: &mut Context<Self>) {
        self.show_changelog = false;
        self.initial_page = Self::WORKSPACE_PAGE;
        self.generation = self.generation.wrapping_add(1);
        cx.notify();
    }

    pub fn open_changelog(&mut self, cx: &mut Context<Self>) {
        self.show_changelog = true;
        self.changelog_version = 0;
        cx.notify();
    }

    fn close_changelog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_changelog = false;
        self.initial_page = Self::ABOUT_PAGE;
        self.generation = self.generation.wrapping_add(1);
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn appearance_page() -> SettingPage {
        let mode = SettingItem::new(
            "Mode",
            SettingField::render(|_, _, cx| {
                segmented(
                    "set-theme",
                    &[
                        (ThemePreference::System, "System"),
                        (ThemePreference::Light, "Light"),
                        (ThemePreference::Dark, "Dark"),
                    ],
                    appearance::settings(cx).theme,
                    |v, _, cx| appearance::update(cx, |s| s.theme = v),
                )
            }),
        )
        .description("System follows your computer's light or dark setting.")
        .keywords(["theme", "dark mode", "light mode", "appearance"]);

        let previews = SettingItem::render(|_, _, cx| {
            h_flex()
                .flex_wrap()
                .gap_3()
                .child(theme_cards(ThemeMode::Light, cx))
                .child(theme_cards(ThemeMode::Dark, cx))
                .into_any_element()
        })
        .keywords(["theme", "light", "dark", "colors", "ottero"]);

        let ui_font = SettingItem::new(
            "Interface",
            SettingField::render(|_, _, cx| {
                h_flex()
                    .gap_2()
                    .child(segmented(
                        "set-ui-font",
                        &[(UiFont::Inter, "Inter"), (UiFont::System, "System")],
                        appearance::settings(cx).ui_font,
                        |v, _, cx| appearance::update(cx, |s| s.ui_font = v),
                    ))
                    .child(size_stepper("ui-size", appearance::settings(cx).ui_font_size, AppearanceSettings::UI_FONT_SIZES, |s, v| s.ui_font_size = v))
            }),
        )
        .description(format!(
            "Font for everything outside code. The size also scales spacing, so it works as zoom. Default {} px.",
            AppearanceSettings::DEFAULT_UI_FONT_SIZE
        ))
        .keywords(["typography", "font", "zoom", "text size", "font size"]);

        let mono_font = SettingItem::new(
            "Code",
            SettingField::render(|_, _, cx| {
                h_flex()
                    .gap_2()
                    .child(segmented(
                        "set-mono-font",
                        &[
                            (MonoFont::JetbrainsMono, "JetBrains Mono"),
                            (MonoFont::System, "System"),
                        ],
                        appearance::settings(cx).mono_font,
                        |v, _, cx| appearance::update(cx, |s| s.mono_font = v),
                    ))
                    .child(size_stepper(
                        "mono-size",
                        appearance::settings(cx).mono_font_size,
                        AppearanceSettings::MONO_FONT_SIZES,
                        |s, v| s.mono_font_size = v,
                    ))
            }),
        )
        .description("Code blocks and inline code in notes.")
        .keywords(["typography", "monospace", "font"]);

        let spacing = SettingItem::new(
            "Line spacing",
            SettingField::render(|_, _, cx| {
                segmented(
                    "set-line-spacing",
                    &[
                        (LineSpacing::Compact, "Compact"),
                        (LineSpacing::Comfortable, "Comfortable"),
                        (LineSpacing::Relaxed, "Relaxed"),
                    ],
                    appearance::settings(cx).line_spacing,
                    |v, _, cx| appearance::update(cx, |s| s.line_spacing = v),
                )
            }),
        )
        .description("Space between lines when writing notes.")
        .keywords(["typography", "line height", "leading", "density"]);

        let preview = SettingItem::render(|_, _, cx| {
            let theme = cx.theme();
            let ratio = appearance::settings(cx).line_spacing.ratio();
            v_flex()
                .w_full()
                .gap_2()
                .p_4()
                .rounded(theme.radius)
                .bg(theme.background)
                .border_1()
                .border_color(theme.border)
                .child(
                    div()
                        .text_lg()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child("Weekend plans"),
                )
                .child(
                    div()
                        .line_height(gpui_kit::relative(ratio))
                        .child("Pack the tent and check the weather on Friday. The quick brown fox jumps over the lazy dog, and the line keeps going so you can see how it wraps."),
                )
                .child(
                    div()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(theme.mono_font_size)
                        .text_color(theme.muted_foreground)
                        .child("fn main() { println!(\"0O 1lI\"); }"),
                )
                .into_any_element()
        })
        .keywords(["preview", "typography"]);

        let width = SettingItem::new(
            "Content width",
            SettingField::render(|_, _, cx| {
                segmented(
                    "set-width",
                    &[
                        (ContentWidth::Compact, "Compact"),
                        (ContentWidth::Comfortable, "Comfortable"),
                        (ContentWidth::Wide, "Wide"),
                    ],
                    appearance::settings(cx).content_width,
                    |v, _, cx| appearance::update(cx, |s| s.content_width = v),
                )
            }),
        )
        .description("How wide notes and lists can grow on large screens.")
        .keywords(["layout", "width", "density"]);

        let motion = SettingItem::new(
            "Reduce motion",
            SettingField::render(|_, _, cx| {
                segmented(
                    "set-motion",
                    &[
                        (MotionPreference::System, "System"),
                        (MotionPreference::On, "On"),
                        (MotionPreference::Off, "Off"),
                    ],
                    appearance::settings(cx).reduce_motion,
                    |v, _, cx| appearance::update(cx, |s| s.reduce_motion = v),
                )
            }),
        )
        .description(
            "Turns off menu and dialog animations. System follows your accessibility setting.",
        )
        .keywords(["animation", "accessibility"]);

        let reset = SettingItem::render(|_, _, _| {
            h_flex()
                .w_full()
                .justify_start()
                .child(
                    Button::new("appearance-reset")
                        .ghost()
                        .small()
                        .label("Restore default appearance")
                        .on_click(|_, _, cx| appearance::reset(cx)),
                )
                .into_any_element()
        })
        .keywords(["reset", "default"]);

        SettingPage::new("Appearance")
            .icon(Lucide::Palette)
            .group(
                SettingGroup::new()
                    .title("Theme")
                    .items(vec![mode, previews]),
            )
            .group(
                SettingGroup::new()
                    .title("Text")
                    .items(vec![ui_font, mono_font, spacing, preview]),
            )
            .group(SettingGroup::new().title("Layout").items(vec![width]))
            .group(
                SettingGroup::new()
                    .title("Accessibility")
                    .items(vec![motion]),
            )
            .group(
                SettingGroup::new()
                    .variant(GroupBoxVariant::Normal)
                    .item(reset),
            )
    }

    fn notes_group() -> SettingGroup {
        let spelling = SettingItem::new(
            "Check spelling",
            SettingField::switch(
                |cx| appearance::settings(cx).spell_check,
                |on, cx| appearance::update(cx, |s| s.spell_check = on),
            ),
        )
        .description(
            "Underlines misspelled words in notes. Right-click one for suggestions. Code, links and tags are skipped.",
        )
        .keywords(["spelling", "spellcheck", "typo", "dictionary"]);

        SettingGroup::new().title("Notes").item(spelling)
    }

    fn privacy_page(data_dir: PathBuf, library: Entity<crate::library::Library>) -> SettingPage {
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
            .groups(Self::password_groups(library))
            .group(
                SettingGroup::new()
                    .title("Storage")
                    .item(location)
                    .footer(|_, _| "Everything stays on this device. Sync is not set up yet."),
            )
    }

    fn today_group() -> SettingGroup {
        use crate::views::today_layout::{TodaySection, moved, resolve};
        let rows = SettingItem::render(|_, _, cx| {
            let prefs = appearance::settings(cx);
            let sections = resolve(&prefs.today_order, &prefs.today_hidden);
            let last = sections.len().saturating_sub(1);
            let theme = cx.theme();
            v_flex()
                .w_full()
                .children(
                    sections
                        .into_iter()
                        .enumerate()
                        .map(|(i, (section, shown))| {
                            let key = section.key();
                            h_flex()
                                .w_full()
                                .gap_3()
                                .py_2()
                                .items_center()
                                .when(i > 0, |s| s.border_t_1().border_color(theme.border))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .text_sm()
                                                .when(!shown, |s| {
                                                    s.text_color(theme.muted_foreground)
                                                })
                                                .child(section.title()),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child(section.description()),
                                        ),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("today-up-{key}")))
                                        .ghost()
                                        .xsmall()
                                        .icon(Lucide::ChevronUp)
                                        .tooltip("Move up")
                                        .disabled(i == 0)
                                        .on_click(move |_, _, cx| {
                                            appearance::update(cx, |s| {
                                                s.today_order = moved(&s.today_order, section, -1)
                                            })
                                        }),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("today-down-{key}")))
                                        .ghost()
                                        .xsmall()
                                        .icon(Lucide::ChevronDown)
                                        .tooltip("Move down")
                                        .disabled(i == last)
                                        .on_click(move |_, _, cx| {
                                            appearance::update(cx, |s| {
                                                s.today_order = moved(&s.today_order, section, 1)
                                            })
                                        }),
                                )
                                .child(
                                    gpui_kit::component::switch::Switch::new(SharedString::from(
                                        format!("today-show-{key}"),
                                    ))
                                    .checked(shown)
                                    .accessibility_label(format!("Show {}", section.title()))
                                    .on_click(
                                        move |on, _, cx| {
                                            crate::views::today::set_shown(section, *on, cx)
                                        },
                                    ),
                                )
                        }),
                )
                .into_any_element()
        })
        .keywords([
            "today",
            "dashboard",
            "sections",
            "order",
            "hide",
            "customize",
        ]);
        let reset = SettingItem::render(|_, _, _| {
            h_flex()
                .w_full()
                .child(
                    Button::new("today-reset")
                        .ghost()
                        .small()
                        .label("Restore default layout")
                        .on_click(|_, _, cx| {
                            appearance::update(cx, |s| {
                                s.today_order.clear();
                                s.today_hidden.clear();
                            })
                        }),
                )
                .into_any_element()
        });
        let _ = TodaySection::ALL;
        SettingGroup::new()
            .title("Today")
            .items(vec![rows, reset])
            .footer(|_, _| "Half-width sections sit side by side on wide windows.")
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
        // What opens Passwords: the master password (with an offer to use the
        // Ottero account when signed in), or the linked account (ADR 0014).
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
                                window.dispatch_action(
                                    Box::new(crate::actions::ChangeMasterPassword),
                                    cx,
                                )
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
                                window.dispatch_action(
                                    Box::new(crate::actions::RestoreVaultBackup),
                                    cx,
                                )
                            }),
                    )
                    .child(
                        Button::new("set-vault-export")
                            .small()
                            .label("Export…")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(
                                    Box::new(crate::actions::ExportVaultBackup),
                                    cx,
                                )
                            }),
                    )
            }),
        )
        .description("A copy of your vault that stays encrypted. It opens with the master password you have when you save it, or with your Ottero account or recovery key if Passwords uses your account.")
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

    fn calendar_groups(
        holidays: &crate::views::holiday_settings::HolidayPickers,
    ) -> Vec<SettingGroup> {
        let week = SettingItem::new(
            "Week starts on",
            SettingField::render(|_, _, cx| {
                segmented(
                    "set-week-start",
                    &[(WeekStart::Monday, "Monday"), (WeekStart::Sunday, "Sunday")],
                    appearance::settings(cx).week_start,
                    |v, _, cx| appearance::update(cx, |s| s.week_start = v),
                )
            }),
        )
        .description("The first column of the month and week views.")
        .keywords(["calendar", "week", "monday", "sunday"]);
        let clock = SettingItem::new(
            "Time format",
            SettingField::render(|_, _, cx| {
                segmented(
                    "set-clock",
                    &[
                        (ClockFormat::TwelveHour, "12-hour"),
                        (ClockFormat::TwentyFourHour, "24-hour"),
                    ],
                    appearance::settings(cx).clock,
                    |v, _, cx| appearance::update(cx, |s| s.clock = v),
                )
            }),
        )
        .description("How times are written: 2:30 PM or 14:30.")
        .keywords(["calendar", "clock", "time", "24 hour", "am pm"]);
        let routines = SettingItem::new(
            "Show routines",
            SettingField::switch(
                |cx| appearance::settings(cx).calendar_routines,
                |on, cx| appearance::update(cx, |s| s.calendar_routines = on),
            ),
        )
        .description(
            "Events that repeat 3 or more times a week. In the month view they show as small dots.",
        )
        .keywords(["calendar", "routine", "repeat", "noise", "hide"]);
        vec![
            SettingGroup::new()
                .title("Calendar")
                .items(vec![week, clock, routines]),
            crate::views::holiday_settings::holiday_group(holidays),
        ]
    }

    fn keyboard_page() -> SettingPage {
        let key_row = |keys: &'static str, label: &'static str| {
            SettingItem::new(
                label,
                SettingField::render(move |_: &RenderOptions, _: &mut Window, _: &mut App| {
                    match Keystroke::parse(keys) {
                        // Kbd draws the platform's symbols: ⌘⇧N on macOS, Ctrl+Shift+N on Windows.
                        Ok(ks) => Kbd::new(ks).into_any_element(),
                        Err(_) => div().child(keys).into_any_element(),
                    }
                }),
            )
            .keywords(["shortcut", "hotkey", "keyboard"])
        };

        let mut groups: Vec<SettingGroup> = ShortcutGroup::ALL
            .into_iter()
            .map(|group| {
                let items = SHORTCUTS
                    .iter()
                    .filter(|s| s.group == group)
                    .map(|s| key_row(s.keys, s.label))
                    .collect::<Vec<_>>();
                SettingGroup::new().title(group.title()).items(items)
            })
            .collect();

        groups.push(
            SettingGroup::new().title("Passwords").items(
                crate::actions::VAULT_SHORTCUTS
                    .iter()
                    .map(|s| key_row(s.keys, s.label))
                    .collect::<Vec<_>>(),
            ),
        );
        groups.push(
            SettingGroup::new().title("Habits").items(
                HABIT_SHORTCUTS
                    .iter()
                    .map(|s| key_row(s.keys, s.label))
                    .collect::<Vec<_>>(),
            ),
        );
        groups.push(
            SettingGroup::new().title("Focus").items(
                crate::actions::FOCUS_SHORTCUTS
                    .iter()
                    .map(|s| key_row(s.keys, s.label))
                    .collect::<Vec<_>>(),
            ),
        );
        groups.push(
            SettingGroup::new().title("Calendar").items(
                CALENDAR_SHORTCUTS
                    .iter()
                    .map(|s| key_row(s.keys, s.label))
                    .collect::<Vec<_>>(),
            ),
        );

        let link = SettingItem::new(
            "Link to another note",
            SettingField::render(|_: &RenderOptions, _: &mut Window, cx: &mut App| {
                div()
                    .px_1p5()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().muted)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_sm()
                    .child("[[")
            }),
        )
        .description("Type it in a note, then pick a title.")
        .keywords(["wiki", "link", "autocomplete"]);
        let mut editing = EDITING_SHORTCUTS
            .iter()
            .map(|&(keys, label)| key_row(keys, label))
            .collect::<Vec<_>>();
        editing.push(link);
        groups.push(
            SettingGroup::new()
                .title("Writing")
                .items(editing)
                .footer(|_, _| {
                    if cfg!(target_os = "macos") {
                        "On Windows and Linux, Ctrl takes the place of ⌘."
                    } else {
                        "On macOS, ⌘ takes the place of Ctrl."
                    }
                }),
        );

        SettingPage::new("Keyboard")
            .icon(Lucide::Keyboard)
            .groups(groups)
    }

    fn about_page() -> SettingPage {
        let version = SettingItem::new(
            "Version",
            SettingField::render(|_, _, _| div().text_sm().child(env!("CARGO_PKG_VERSION"))),
        );
        let fonts = SettingItem::new(
            "Fonts",
            SettingField::render(|_, _, _| {
                div().text_sm().child("Inter, JetBrains Mono · SIL OFL 1.1")
            }),
        );
        let icons = SettingItem::new(
            "Icons",
            SettingField::render(|_, _, _| div().text_sm().child("Lucide · ISC")),
        );
        let maker = SettingItem::new(
            "Made by",
            SettingField::render(|_, _, _| div().text_sm().child("Nexdark Labs")),
        )
        .description("ottero.nexdark.com");
        let header = SettingItem::render(|_, _, cx| {
            h_flex()
                .gap_3()
                .items_center()
                .child(
                    gpui_kit::img(ottero_ui::assets::APP_ICON)
                        .size(gpui_kit::rems(3.5))
                        .flex_shrink_0(),
                )
                .child(
                    v_flex()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .child("Ottero"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Notes, tasks, calendar and passwords, in one calm place."),
                        ),
                )
                .into_any_element()
        });
        let updates = SettingItem::new(
            "Updates",
            SettingField::render(|_, _, cx| {
                use crate::updates::Status;
                let (text, busy) = match crate::updates::status(cx) {
                    Status::Checking => ("Checking…".to_owned(), true),
                    Status::Downloading { version } => (format!("Downloading {version}…"), true),
                    Status::Verifying { version } => (format!("Verifying {version}…"), true),
                    Status::Installing { version } => (format!("Installing {version}…"), true),
                    Status::Ready { version, .. } => {
                        (format!("{version} installs when you restart"), false)
                    }
                    Status::Available { version } => (format!("{version} is available"), false),
                    Status::UpToDate => ("Up to date".to_owned(), false),
                    Status::Unsupported => (
                        "Automatic updates aren't available for this platform".to_owned(),
                        false,
                    ),
                    Status::Failed { message } => (message, false),
                    Status::Idle => (String::new(), false),
                };
                let ready = matches!(crate::updates::status(cx), Status::Ready { .. });
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .id("set-update-status")
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(gpui_kit::component::label::Label::new(text)),
                    )
                    .child(if ready {
                        Button::new("set-update-restart")
                            .small()
                            .primary()
                            .label("Restart to update")
                            .on_click(|_, window, cx| {
                                window
                                    .dispatch_action(Box::new(crate::actions::RestartToUpdate), cx)
                            })
                    } else {
                        Button::new("set-update-check")
                            .small()
                            .label("Check now")
                            .disabled(busy)
                            .on_click(|_, window, cx| {
                                window
                                    .dispatch_action(Box::new(crate::actions::CheckForUpdates), cx)
                            })
                    })
                    .into_any_element()
            }),
        )
        .description("New versions are signed by Nexdark Labs and checked before they install.")
        .keywords(["update", "upgrade", "new version", "version"]);
        let changelog = SettingItem::new(
            "What’s new",
            SettingField::render(|_, _, _| {
                Button::new("set-whats-new")
                    .small()
                    .label("Read release notes")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::ShowChangelog), cx)
                    })
            }),
        )
        .description("New features, improvements and fixes, across every release.")
        .keywords(["changelog", "release notes", "changes", "what’s new"]);
        let automatic = SettingItem::new(
            "Check for updates automatically",
            SettingField::switch(crate::updates::automatic, crate::updates::set_automatic),
        )
        .description(
            "Once a day. Updates download in the background and install only when you restart.",
        )
        .keywords(["update", "automatic", "background"]);
        SettingPage::new("About").icon(Lucide::Info).group(
            SettingGroup::new().title("Ottero").items(vec![
                header, version, updates, automatic, changelog, maker, fonts, icons,
            ]),
        )
    }
}

/// A compact "− 15 px +" control for font sizes.
/// One row of theme cards for `mode`. Each card previews the theme in its own
/// colors (page, text, accent, two code colors); the chosen one has a ring.
/// Choosing a theme of the other mode also switches to that mode, so the
/// change is visible, unless the mode follows the system.
fn theme_cards(mode: ThemeMode, cx: &App) -> impl IntoElement {
    use gpui_kit::component::try_parse_color;
    let app = cx.theme();
    let settings = appearance::settings(cx);
    let selected = if mode.is_dark() {
        settings.dark_theme.clone()
    } else {
        settings.light_theme.clone()
    };
    let cards = ottero_ui::theme::choices(mode, cx)
        .into_iter()
        .map(|choice| {
            let c = &choice.config.colors;
            let parse = |v: &Option<SharedString>| v.as_ref().and_then(|s| try_parse_color(s).ok());
            let bg = parse(&c.background).unwrap_or(app.background);
            let fg = parse(&c.foreground).unwrap_or(app.foreground);
            let border = parse(&c.border).unwrap_or(app.border);
            let accent = parse(&c.primary).unwrap_or(fg);
            let syntax = choice.config.highlight.as_ref().map(|h| &h.syntax);
            let tone =
                |s: Option<&gpui_kit::component::highlighter::ThemeStyle>| s.and_then(|s| s.color);
            let dots = [
                accent,
                syntax
                    .and_then(|s| tone(s.keyword.as_ref()))
                    .unwrap_or(accent),
                syntax.and_then(|s| tone(s.string.as_ref())).unwrap_or(fg),
                syntax.and_then(|s| tone(s.function.as_ref())).unwrap_or(fg),
            ];
            let is_selected = choice.name.as_ref() == selected;
            let name = choice.name.clone();
            let id = SharedString::from(format!(
                "theme-{}",
                choice.name.to_lowercase().replace(' ', "-")
            ));
            v_flex()
                .id(gpui_kit::ElementId::Name(id))
                .test_support()
                .w(gpui_kit::rems(9.))
                .gap_1p5()
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    let name = name.to_string();
                    appearance::update(cx, |s| {
                        if mode.is_dark() {
                            s.dark_theme = name;
                        } else {
                            s.light_theme = name;
                        }
                        if s.theme != ThemePreference::System {
                            s.theme = if mode.is_dark() {
                                ThemePreference::Dark
                            } else {
                                ThemePreference::Light
                            };
                        }
                    });
                })
                .child(
                    v_flex()
                        .h(gpui_kit::rems(5.))
                        .p_2()
                        .gap_1()
                        .justify_between()
                        .rounded(app.radius_lg)
                        .bg(bg)
                        .border_1()
                        .border_color(border)
                        .when(is_selected, |this| this.border_2().border_color(app.ring))
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(fg)
                                .child("Aa"),
                        )
                        .child(
                            h_flex().gap_1().children(
                                dots.into_iter()
                                    .map(|d| div().size(gpui_kit::rems(0.6)).rounded_full().bg(d)),
                            ),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if is_selected {
                            app.foreground
                        } else {
                            app.muted_foreground
                        })
                        .when(is_selected, |this| {
                            this.font_weight(gpui_kit::FontWeight::MEDIUM)
                        })
                        .truncate()
                        .child(choice.name.clone()),
                )
        });
    h_flex().w_full().flex_wrap().gap_3().children(cards)
}

fn size_stepper(
    id: &'static str,
    value: u8,
    range: std::ops::RangeInclusive<u8>,
    set: fn(&mut AppearanceSettings, u8),
) -> impl IntoElement {
    let (min, max) = (*range.start(), *range.end());
    h_flex()
        .gap_1()
        .child(
            Button::new(SharedString::from(format!("{id}-dec")))
                .outline()
                .xsmall()
                .icon(Lucide::Minus)
                .disabled(value <= min)
                .tooltip("Smaller")
                .on_click(move |_, _, cx| {
                    appearance::update(cx, |s| set(s, value.saturating_sub(1).max(min)))
                }),
        )
        .child(
            div()
                .w_12()
                .text_center()
                .text_sm()
                .child(format!("{value} px")),
        )
        .child(
            Button::new(SharedString::from(format!("{id}-inc")))
                .outline()
                .xsmall()
                .icon(Lucide::Plus)
                .disabled(value >= max)
                .tooltip("Larger")
                .on_click(move |_, _, cx| {
                    appearance::update(cx, |s| set(s, value.saturating_add(1).min(max)))
                }),
        )
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.show_changelog {
            return super::changelog::render(
                &self.focus,
                self.changelog_version,
                f32::from(window.viewport_size().width) / f32::from(window.rem_size()) < 72.,
                {
                    let view = cx.entity().downgrade();
                    move |index, _, cx| {
                        _ = view.update(cx, |this, cx| {
                            this.changelog_version = index;
                            cx.notify();
                        });
                    }
                },
                cx.listener(|this, _, window, cx| this.close_changelog(window, cx)),
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        this.close_changelog(window, cx);
                        cx.stop_propagation();
                    }
                }),
                cx,
            );
        }
        match &mut self.holiday_pickers {
            Some(p) => p.sync(window, cx),
            None => {
                self.holiday_pickers = Some(crate::views::holiday_settings::pickers(window, cx));
            }
        }
        let Some(pickers) = self.holiday_pickers.as_ref() else {
            return div();
        };
        div().track_focus(&self.focus).size_full().child(
            Settings::new(("ottero-settings", self.generation))
                .sidebar_width(window.rem_size() * 13.)
                .default_selected_index(gpui_kit::component::setting::SelectIndex {
                    page_ix: self.initial_page,
                    group_ix: None,
                })
                .with_group_variant(GroupBoxVariant::Outline)
                .pages(vec![
                    super::account::page(&self.account, cx),
                    super::workspace_settings::general_page(self.layout.clone()),
                    super::workspace_settings::workspace_page(
                        self.layout.clone(),
                        &self.reorder_focus,
                        cx,
                    ),
                    Self::appearance_page(),
                    SettingPage::new("Features")
                        .icon(Lucide::SlidersHorizontal)
                        .group(Self::notes_group())
                        .group(Self::today_group())
                        .groups(Self::calendar_groups(pickers)),
                    Self::privacy_page(
                        self.data_dir.clone(),
                        self.account.read(cx).library().clone(),
                    ),
                    Self::keyboard_page(),
                    Self::about_page(),
                ]),
        )
    }
}
