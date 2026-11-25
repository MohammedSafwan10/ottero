//! Feature preference groups: Notes, Today dashboard layout, and Calendar.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{IntoElement, ParentElement as _, SharedString, Styled as _, div};
use ottero_ui::appearance::{self, ClockFormat, WeekStart};
use ottero_ui::components::segmented;

pub(super) fn notes_group() -> SettingGroup {
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

pub(super) fn today_group() -> SettingGroup {
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
                                            .when(!shown, |s| s.text_color(theme.muted_foreground))
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
                                .on_click(move |on, _, cx| {
                                    crate::views::today::set_shown(section, *on, cx)
                                }),
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

pub(super) fn calendar_groups(
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
