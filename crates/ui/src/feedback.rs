//! Consistent feedback for actions: confirmations, undo, and errors.
//!
//! Policy (see `docs/design-system.md`):
//! * Reversible actions (move to trash, complete, archive) run immediately and
//!   show an [`undo_toast`]. No confirmation dialog.
//! * Permanent actions (delete forever, empty trash, wipe data) always go
//!   through [`confirm_destructive`].
//! * Failures surface as an [`error_toast`], never silently.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::notification::{Notification, NotificationType};
use gpui_kit::component::{Sizable as _, WindowExt as _};
use gpui_kit::{App, SharedString, Window};

/// Text for a destructive confirmation. Name the object and the consequence.
pub struct Confirm {
    pub title: SharedString,
    pub description: SharedString,
    /// The verb on the button, e.g. "Delete" or "Empty trash". Never "OK".
    pub confirm_label: SharedString,
}

impl Confirm {
    #[must_use]
    pub fn new(
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        confirm_label: impl Into<SharedString>,
    ) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            confirm_label: confirm_label.into(),
        }
    }
}

/// Asks before a permanent action. `on_confirm` runs only if the user confirms.
/// Cancel is the default-safe choice; Escape cancels.
pub fn confirm_destructive(
    confirm: Confirm,
    window: &mut Window,
    cx: &mut App,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
) {
    // One question at a time: a double-click must not stack two confirmations.
    if window.has_active_dialog(cx) {
        return;
    }
    let on_confirm = Rc::new(on_confirm);
    window.open_alert_dialog(cx, move |dialog, _, _| {
        let on_confirm = on_confirm.clone();
        dialog
            .title(confirm.title.clone())
            .description(confirm.description.clone())
            .confirm()
            .ok_text(confirm.confirm_label.clone())
            .ok_variant(ButtonVariant::Danger)
            .cancel_text("Cancel")
            .on_ok(move |_, window, cx| {
                on_confirm(window, cx);
                true
            })
    });
}

struct InfoToast;
struct UndoToast;
struct ErrorToast;

/// Shows a calm, deduplicated status toast (identical messages replace in place).
pub fn info_toast(message: impl Into<SharedString>, window: &mut Window, cx: &mut App) {
    let message: SharedString = message.into();
    window.push_notification(
        Notification::new()
            .id1::<InfoToast>(message.clone())
            .message(message),
        cx,
    );
}

/// Confirms a reversible action with an Undo button. Hides itself after a few seconds.
pub fn undo_toast(
    message: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
    on_undo: impl Fn(&mut Window, &mut App) + 'static,
) {
    let on_undo = Rc::new(on_undo);
    let message: SharedString = message.into();
    let notification = Notification::new()
        .id1::<UndoToast>(message.clone())
        .message(message)
        .action(move |_, _, cx| {
            let on_undo = on_undo.clone();
            Button::new("undo")
                .outline()
                .xsmall()
                .label("Undo")
                .on_click(cx.listener(move |this, _, window, cx| {
                    on_undo(window, cx);
                    this.dismiss(window, cx);
                }))
        })
        // `action` turns autohide off; an undo offer should still go away by itself.
        .autohide(true);
    window.push_notification(notification, cx);
}

/// Shows a failure. Messages come from domain/storage errors and are user-safe.
pub fn error_toast(message: impl Into<SharedString>, window: &mut Window, cx: &mut App) {
    let message: SharedString = message.into();
    window.push_notification(
        Notification::new()
            .id1::<ErrorToast>(message.clone())
            .with_type(NotificationType::Error)
            .message(message),
        cx,
    );
}

/// Shows `result`'s error, if any, as an [`error_toast`].
pub fn report<T, E: std::fmt::Display>(
    result: Result<T, E>,
    window: &mut Window,
    cx: &mut App,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(err) => {
            error_toast(format!("{err:#}"), window, cx);
            None
        }
    }
}

/// Asks whether a change to a repeating event is for this occurrence only
/// or the whole series (Apple Calendar's question). Cancel and Escape do
/// nothing; `on_choose(true)` means "only this one".
pub fn ask_repeat_scope(
    title: impl Into<SharedString>,
    this_label: impl Into<SharedString>,
    all_label: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
    on_choose: impl Fn(bool, &mut Window, &mut App) + 'static,
) {
    use gpui_kit::component::dialog::DialogFooter;
    use gpui_kit::{ParentElement as _, Styled as _};

    if window.has_active_dialog(cx) {
        return;
    }
    let on_choose = Rc::new(on_choose);
    let (title, this_label, all_label): (SharedString, SharedString, SharedString) =
        (title.into(), this_label.into(), all_label.into());
    window.open_dialog(cx, move |dialog, _, cx| {
        let (this, all) = (on_choose.clone(), on_choose.clone());
        dialog
            .title(title.clone())
            .w(gpui_kit::px(420.))
            .child(
                gpui_kit::div()
                    .text_sm()
                    .text_color(gpui_kit::component::ActiveTheme::theme(cx).muted_foreground)
                    .child("This is a repeating event."),
            )
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("scope-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("scope-all")
                            .outline()
                            .label(all_label.clone())
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                all(false, window, cx);
                            }),
                    )
                    .child(
                        Button::new("scope-this")
                            .primary()
                            .label(this_label.clone())
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                this(true, window, cx);
                            }),
                    ),
            )
    });
}

/// Asks for one line of text ("New project", "Rename"). Enter or the confirm
/// button submits; empty input is ignored; Escape cancels.
pub fn prompt_text(
    title: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
    initial: impl Into<SharedString>,
    confirm_label: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
    on_submit: impl Fn(String, &mut Window, &mut App) + 'static,
) {
    use gpui_kit::component::dialog::DialogFooter;
    use gpui_kit::component::input::{Input, InputEvent, InputState};
    use gpui_kit::{AppContext as _, ParentElement as _};

    if window.has_active_dialog(cx) {
        return;
    }
    let initial: SharedString = initial.into();
    let placeholder: SharedString = placeholder.into();
    let input = cx.new(|cx| {
        let mut state = InputState::new(window, cx).placeholder(placeholder);
        state.set_value(initial.clone(), window, cx);
        state
    });
    /// Receives the trimmed, non-empty value.
    type OnSubmit = Rc<dyn Fn(String, &mut Window, &mut App)>;
    /// Returns `true` when the value was accepted and the dialog may close.
    type Submit = Rc<dyn Fn(&mut Window, &mut App) -> bool>;

    let on_submit: OnSubmit = Rc::new(on_submit);
    let submit: Submit = {
        let input = input.clone();
        Rc::new(move |window, cx| {
            let value = input.read(cx).value().trim().to_owned();
            if value.is_empty() {
                return false;
            }
            on_submit(value, window, cx);
            true
        })
    };
    // Enter in the field submits too. The subscription lives as long as the input.
    let enter = submit.clone();
    let window_handle = window.window_handle();
    cx.subscribe(&input, move |_, event: &InputEvent, cx| {
        if matches!(event, InputEvent::PressEnter { .. }) {
            let enter = enter.clone();
            _ = window_handle.update(cx, move |_, window, cx| {
                if enter(window, cx) {
                    window.close_dialog(cx);
                }
            });
        }
    })
    .detach();

    let title: SharedString = title.into();
    let confirm_label: SharedString = confirm_label.into();
    let field = input.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let submit = submit.clone();
        dialog
            .title(title.clone())
            .w(gpui_kit::px(420.))
            .child(Input::new(&field))
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("prompt-cancel")
                            .outline()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("prompt-ok")
                            .primary()
                            .label(confirm_label.clone())
                            .on_click(move |_, window, cx| {
                                if submit(window, cx) {
                                    window.close_dialog(cx);
                                }
                            }),
                    ),
            )
    });
    input.update(cx, |state, cx| state.focus(window, cx));
}
