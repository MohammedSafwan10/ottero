//! Appearance settings page: theme mode, Ottero Light/Dark previews, fonts,
//! content width, and reduced motion.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::ThemeMode;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use ottero_ui::appearance::{
    self, AppearanceSettings, ContentWidth, LineSpacing, MonoFont, MotionPreference,
    ThemePreference, UiFont,
};
use ottero_ui::components::segmented;

pub(super) fn appearance_page() -> SettingPage {
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
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(segmented(
                    "set-ui-font",
                    &[(UiFont::Inter, "Inter"), (UiFont::System, "System")],
                    appearance::settings(cx).ui_font,
                    |v, _, cx| appearance::update(cx, |s| s.ui_font = v),
                ))
                .child(size_stepper(
                    "ui-size",
                    appearance::settings(cx).ui_font_size,
                    AppearanceSettings::UI_FONT_SIZES,
                    |s, v| s.ui_font_size = v,
                ))
        }),
    )
    .layout(gpui_kit::Axis::Vertical)
    .description(format!(
        "Font for everything outside code. The size also scales spacing, so it works as zoom. Default {} px.",
        AppearanceSettings::DEFAULT_UI_FONT_SIZE
    ))
    .keywords(["typography", "font", "zoom", "text size", "font size"]);

    let mono_font = SettingItem::new(
        "Code",
        SettingField::render(|_, _, cx| {
            h_flex()
                .flex_wrap()
                .items_center()
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
    .layout(gpui_kit::Axis::Vertical)
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
    .layout(gpui_kit::Axis::Vertical)
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
    .layout(gpui_kit::Axis::Vertical)
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
    .description("Turns off menu and dialog animations. System follows your accessibility setting.")
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

/// Visual preview cards for `mode`. Clicking a card selects that mode so the
/// palette change is immediately visible.
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
            let is_selected = choice.name.as_ref() == selected && app.mode == mode;
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
                        s.theme = if mode.is_dark() {
                            ThemePreference::Dark
                        } else {
                            ThemePreference::Light
                        };
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
    h_flex().flex_wrap().gap_3().children(cards)
}

/// A compact "− 15 px +" control for font sizes.
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
