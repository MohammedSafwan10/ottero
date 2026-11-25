//! Ottero design system.
//!
//! * [`theme`]: the monochrome Ottero Light/Dark themes (`themes/ottero.json`).
//! * [`fonts`]: bundled Inter and JetBrains Mono (SIL OFL 1.1).
//! * [`appearance`]: user-chosen appearance (mode, fonts, sizes, motion,
//!   content width), persisted as JSON and applied to the GPUI Kit theme.
//! * [`components`]: small reusable pieces shared by every feature screen.
//!
//! Feature code must read colors from `cx.theme()` and never hard-code them.

pub mod appearance;
pub mod assets;
pub mod components;
pub mod feedback;
pub mod fonts;
pub mod panel;
pub mod theme;

use std::path::PathBuf;

use gpui_kit::App;

/// Installs fonts, themes, and the saved appearance. Call once after
/// `gpui_kit::init` and before opening any window.
pub fn init(appearance_file: PathBuf, cx: &mut App) {
    fonts::register(cx);
    theme::register(cx);
    appearance::init(appearance_file, cx);
    // Toasts go bottom-right: the top-right is where every screen keeps its
    // toolbar actions, and a toast there hides them for seconds. Cap the stack
    // at 3 and keep toasts compact so narrower windows stay uncluttered.
    gpui_kit::component::Theme::update(cx, |theme| {
        theme.notification.placement = gpui_kit::Anchor::BottomRight;
        theme.notification.max_items = 3;
        theme.notification.width = gpui_kit::px(340.);
    });
}
