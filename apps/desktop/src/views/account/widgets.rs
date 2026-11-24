//! Shared UI building blocks and password confirmation prompt for Account settings.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Window, div, px, rems,
};
use ottero_crypto::SecretString;

use crate::library::Library;
use crate::views::vault_item::copy_field;

pub(super) fn icon_tile(icon: Lucide, cx: &App) -> impl IntoElement {
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

pub(super) fn label(text: &'static str, cx: &App) -> impl IntoElement {
    let _ = cx;
    div().text_sm().font_weight(FontWeight::MEDIUM).child(text)
}

pub(super) fn note(text: SharedString, error: bool, cx: &App) -> AnyElement {
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

pub(super) fn field(
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

pub(super) fn divider(cx: &App) -> impl IntoElement {
    div().w_full().h(px(1.)).bg(cx.theme().border)
}

pub(super) fn row_text(
    title: &'static str,
    description: &'static str,
    cx: &App,
) -> impl IntoElement {
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

pub(super) fn info_row(
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
pub(super) fn value_row(
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
pub(super) fn open_password_prompt(
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
