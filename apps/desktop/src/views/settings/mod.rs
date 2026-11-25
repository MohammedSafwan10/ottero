//! Settings screen, built on GPUI Kit's searchable `Settings` component.
//!
//! Pages: Account, General, Workspace, Appearance, Features, Privacy & data, Keyboard,
//! About. Feature preferences stay grouped under Features; search covers all pages.

mod appearance;
mod features;
mod privacy;

use std::path::PathBuf;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::setting::{
    RenderOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, InteractiveElement as _, IntoElement, Keystroke,
    ParentElement as _, Render, Styled as _, Subscription, TestSupportExt as _, Window, div,
};

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
                    .w_full()
                    .flex_wrap()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .id("set-update-status")
                            .test_support()
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
        .layout(gpui_kit::Axis::Vertical)
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
                    appearance::appearance_page(),
                    SettingPage::new("Features")
                        .icon(Lucide::SlidersHorizontal)
                        .group(features::notes_group())
                        .group(features::today_group())
                        .groups(features::calendar_groups(pickers)),
                    privacy::privacy_page(
                        self.data_dir.clone(),
                        self.account.read(cx).library().clone(),
                    ),
                    Self::keyboard_page(),
                    Self::about_page(),
                ]),
        )
    }
}
