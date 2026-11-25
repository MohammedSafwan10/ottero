//! The right pane of Passwords: one item, read or edit.
//!
//! Read mode shows fields as rows in one card: secrets masked until
//! revealed, every value copyable (secrets clear from the clipboard after
//! the delay in Settings → Passwords), a live one-time code with its
//! countdown, websites that open in the browser, notes, tags, health issues,
//! and the previous passwords. Edit mode uses the same layout with fields;
//! the password field has a generator. Leaving edit (Save, selecting another
//! item, locking) saves; Cancel or Esc throws the draft away.
//!
//! Nothing secret is ever logged; drafts are zeroed when dropped.

use std::collections::HashSet;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::progress::ProgressCircle;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Rems, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task as GpuiTask, Window, div,
    rems,
};
use ottero_core::VaultItemId;
use ottero_core::vault::health::estimate;
use ottero_core::vault::template::Template;
use ottero_core::vault::{
    CustomField, FieldKind, ItemFields, ItemKind, VaultItem, host, openable_url, totp,
};
use ottero_crypto::Zeroizing;
use ottero_ui::components::{empty_state, section_label};
use ottero_ui::feedback::{self, Confirm};

use super::environment::EnvironmentEditor;
use crate::library::Library;
use crate::vault_prefs;
use crate::views::passwords::{kind_icon, strength_meter, unix_now};
use crate::views::task_row::notify_err;

/// Uniform random `0..n` from the OS CSPRNG, `None` if it failed (the
/// generator then refuses to produce anything).
pub fn random_below(n: u32) -> Option<u32> {
    ottero_crypto::random_below(n).ok()
}

/// Shown when the OS random source fails.
pub const RANDOM_FAILED: &str =
    "Couldn't get secure random numbers from the system, so no password was generated.";

/// Copies `value`. Secrets clear from the clipboard after the configured
/// delay, unless something else was copied since.
pub fn copy_field(label: &str, value: String, secret: bool, window: &mut Window, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(value.clone()));
    struct ClipboardNotice;
    if secret {
        let delay = vault_prefs::get(cx).clipboard.after();
        let value = Zeroizing::new(value);
        cx.spawn(async move |cx| {
            cx.background_executor().timer(delay).await;
            cx.update(|cx| {
                let still_ours = cx
                    .read_from_clipboard()
                    .and_then(|c| c.text())
                    .is_some_and(|t| t == *value);
                if still_ours {
                    cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
                }
            });
        })
        .detach();
        window.push_notification(
            gpui_kit::component::notification::Notification::new()
                .id::<ClipboardNotice>()
                .message(format!("{label} copied. Clears in {} s.", delay.as_secs())),
            cx,
        );
    } else {
        window.push_notification(
            gpui_kit::component::notification::Notification::new()
                .id::<ClipboardNotice>()
                .message(format!("{label} copied.")),
            cx,
        );
    }
}

/// A rounded tile with the item's first letter (or its kind's icon).
pub fn avatar(item: &VaultItem, size: Rems, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let letter = item
        .title()
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string());
    div()
        .size(size)
        .flex_shrink_0()
        .rounded(theme.radius)
        .bg(theme.muted)
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.muted_foreground)
        .map(|s| match (item.kind(), letter) {
            (ItemKind::Login, Some(l)) => s.child(l),
            (kind, _) => s.child(Icon::new(kind_icon(kind)).small()),
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneEvent {
    Created(VaultItemId),
    Selected(VaultItemId),
    Removed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Read,
    Edit,
    New(ItemKind),
}

/// One custom field in the edit form.
struct CustomRow {
    kind: FieldKind,
    label: Entity<InputState>,
    value: Entity<InputState>,
}

/// The edit form. One input per field the kind uses.
struct Form {
    title: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    urls: Entity<TextareaState>,
    totp: Entity<InputState>,
    cardholder: Entity<InputState>,
    number: Entity<InputState>,
    expiry: Entity<InputState>,
    cvv: Entity<InputState>,
    pin: Entity<InputState>,
    notes: Entity<TextareaState>,
    tags: Entity<InputState>,
}

impl Form {
    fn new(window: &mut Window, cx: &mut Context<VaultItemPane>) -> Self {
        let field = |placeholder: &'static str,
                     masked: bool,
                     window: &mut Window,
                     cx: &mut Context<VaultItemPane>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(masked)
                    .placeholder(placeholder)
            })
        };
        Self {
            title: field("Title", false, window, cx),
            username: field("Username or email", false, window, cx),
            password: field("Password", true, window, cx),
            urls: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(1, 6)
                    .placeholder("example.com (one per line)")
            }),
            totp: field("Setup key or otpauth:// link", true, window, cx),
            cardholder: field("Name on card", false, window, cx),
            number: field("Card number", true, window, cx),
            expiry: field("MM/YY", false, window, cx),
            cvv: field("Security code", true, window, cx),
            pin: field("PIN", true, window, cx),
            notes: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(3, 16)
                    .placeholder("Notes")
            }),
            tags: field("Tags, separated by commas", false, window, cx),
        }
    }

    fn fill(&self, f: &ItemFields, window: &mut Window, cx: &mut App) {
        let set = |input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App| {
            input.update(cx, |i, cx| i.set_value(value.to_owned(), window, cx));
        };
        set(&self.title, &f.title, window, cx);
        set(&self.username, &f.username, window, cx);
        set(&self.password, &f.password, window, cx);
        self.urls
            .update(cx, |i, cx| i.set_value(f.urls.join("\n"), window, cx));
        set(&self.totp, &f.totp, window, cx);
        set(&self.cardholder, &f.cardholder, window, cx);
        set(&self.number, &f.number, window, cx);
        set(&self.expiry, &f.expiry, window, cx);
        set(&self.cvv, &f.cvv, window, cx);
        set(&self.pin, &f.pin, window, cx);
        self.notes
            .update(cx, |i, cx| i.set_value(f.notes.clone(), window, cx));
        set(&self.tags, &f.tags.join(", "), window, cx);
        for masked in [
            &self.password,
            &self.totp,
            &self.number,
            &self.cvv,
            &self.pin,
        ] {
            masked.update(cx, |i, cx| i.set_masked(true, window, cx));
        }
    }

    fn read(&self, kind: ItemKind, cx: &App) -> ItemFields {
        let v = |input: &Entity<InputState>| input.read(cx).value().to_string();
        ItemFields {
            kind,
            title: v(&self.title),
            username: v(&self.username),
            password: v(&self.password),
            urls: self
                .urls
                .read(cx)
                .value()
                .lines()
                .map(str::to_owned)
                .collect(),
            totp: v(&self.totp),
            cardholder: v(&self.cardholder),
            number: v(&self.number),
            expiry: v(&self.expiry),
            cvv: v(&self.cvv),
            pin: v(&self.pin),
            notes: self.notes.read(cx).value().to_string(),
            tags: v(&self.tags).split(',').map(str::to_owned).collect(),
            custom: Vec::new(),
            template: None,
            environment: None,
        }
    }

    /// Empties every field (so secrets don't linger in input state).
    fn clear(&self, window: &mut Window, cx: &mut App) {
        let empty = ItemFields::default();
        self.fill(&empty, window, cx);
    }
}

pub struct VaultItemPane {
    environment: Entity<EnvironmentEditor>,
    environment_enabled: bool,
    library: Entity<Library>,
    id: Option<VaultItemId>,
    item: Option<VaultItem>,
    mode: Mode,
    form: Form,
    /// Custom fields being edited, in order.
    custom: Vec<CustomRow>,
    /// The template of a new item (its suggested fields and icon).
    template: Option<Template>,
    revealed: HashSet<SharedString>,
    history_open: bool,
    error: Option<SharedString>,
    _ticker: Option<GpuiTask<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PaneEvent> for VaultItemPane {}

impl VaultItemPane {
    pub fn new(library: Entity<Library>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let form = Form::new(window, cx);
        let environment = cx.new(|cx| EnvironmentEditor::new(library.clone(), window, cx));
        let password_changed = cx.subscribe_in(&form.password, window, |_, _, e, _, cx| {
            if matches!(e, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            environment,
            environment_enabled: false,
            library,
            id: None,
            item: None,
            mode: Mode::Read,
            form,
            custom: Vec::new(),
            template: None,
            revealed: HashSet::new(),
            history_open: false,
            error: None,
            _ticker: None,
            _subscriptions: vec![password_changed],
        }
    }

    pub fn is_editing(&self) -> bool {
        self.mode != Mode::Read
    }

    pub fn item_id(&self) -> Option<VaultItemId> {
        self.id
    }

    /// Shows `id` (or nothing). Saving must succeed before replacing a draft.
    pub fn show(&mut self, id: Option<VaultItemId>, window: &mut Window, cx: &mut Context<Self>) {
        let same = id == self.id;
        if !same && self.is_editing() {
            self.save(window, cx);
            if self.is_editing() {
                return;
            }
        }
        if matches!(self.mode, Mode::New(_)) && id.is_none() {
            // A new item stays open while the list refreshes behind it.
            return;
        }
        self.id = id;
        self.item = id.and_then(|id| self.library.read(cx).vault_item(id));
        if self.item.is_none() {
            self.id = None;
            if self.mode == Mode::Edit {
                self.mode = Mode::Read;
            }
        }
        if !same {
            self.revealed.clear();
            self.history_open = false;
            self.error = None;
        }
        if self.mode == Mode::Read {
            let env = self
                .item
                .as_ref()
                .and_then(|i| i.fields().environment.as_ref());
            self.environment_enabled = env.is_some();
            self.environment.update(cx, |editor, cx| {
                editor.fill(env, self.id, false, window, cx)
            });
        }
        self.update_ticker(window, cx);
        cx.notify();
    }

    /// One-time codes change every 30 s: re-render each second while one shows.
    fn update_ticker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let needs =
            self.mode == Mode::Read && self.item.as_ref().is_some_and(|i| i.otp_setup().is_some());
        if needs && self._ticker.is_none() {
            self._ticker = Some(cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }));
        } else if !needs {
            self._ticker = None;
        }
    }

    /// Forgets everything shown or typed (the vault locked). Unlike
    /// `show(None)`, this also throws away an unsaved new item.
    pub fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.environment
            .update(cx, |editor, cx| editor.clear(window, cx));
        self.environment_enabled = false;
        self.form.clear(window, cx);
        self.clear_custom(window, cx);
        self.mode = Mode::Read;
        self.id = None;
        self.item = None;
        self.revealed.clear();
        self.history_open = false;
        self.error = None;
        self._ticker = None;
        cx.notify();
    }

    /// Replaces the custom-field editors with `fields`.
    fn set_custom(&mut self, fields: &[CustomField], window: &mut Window, cx: &mut Context<Self>) {
        self.clear_custom(window, cx);
        for f in fields {
            self.add_custom(f.kind, &f.label, &f.value, window, cx);
        }
    }

    fn add_custom(
        &mut self,
        kind: FieldKind,
        label: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label_input = cx.new(|cx| {
            let mut i = InputState::new(window, cx).placeholder("Label");
            i.set_value(label.to_owned(), window, cx);
            i
        });
        let placeholder = match kind {
            FieldKind::Text => "Value",
            FieldKind::Secret => "Hidden value",
            FieldKind::Url => "example.com",
            FieldKind::Date => "YYYY-MM-DD",
            FieldKind::OneTimeCode => "Setup key or otpauth:// link",
        };
        let value_input = cx.new(|cx| {
            let mut i = InputState::new(window, cx)
                .masked(kind.is_secret())
                .placeholder(placeholder);
            i.set_value(value.to_owned(), window, cx);
            i
        });
        self.custom.push(CustomRow {
            kind,
            label: label_input,
            value: value_input,
        });
        cx.notify();
    }

    /// Empties and drops every custom-field editor.
    fn clear_custom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for row in self.custom.drain(..) {
            for input in [&row.label, &row.value] {
                input.update(cx, |i, cx| i.set_value("", window, cx));
            }
        }
    }

    /// Whether any edit field holds text (tests check drafts are wiped).
    #[doc(hidden)]
    pub fn draft_is_empty(&self, cx: &App) -> bool {
        let f = &self.form;
        [
            &f.title,
            &f.username,
            &f.password,
            &f.totp,
            &f.number,
            &f.cvv,
            &f.pin,
            &f.tags,
        ]
        .iter()
        .all(|i| i.read(cx).value().is_empty())
            && self.custom.is_empty()
            && f.notes.read(cx).value().is_empty()
            && f.urls.read(cx).value().is_empty()
            && self.environment.read(cx).draft_is_empty(cx)
    }

    #[doc(hidden)]
    pub fn type_draft_password_for_tests(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.form
            .password
            .update(cx, |i, cx| i.set_value(value.to_owned(), window, cx));
    }

    pub fn environment_editor(&self) -> &Entity<EnvironmentEditor> {
        &self.environment
    }

    pub fn start_environment(
        &mut self,
        project: ottero_core::ProjectId,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_template(Template::Environment, window, cx);
        let env = ottero_core::vault::environment::EnvironmentSecrets::new(
            Some(project),
            "Development",
            vec![],
        );
        if let Ok(env) = env {
            self.environment.update(cx, |editor, cx| {
                editor.fill(Some(&env), None, true, window, cx)
            });
        }
        self.form.title.update(cx, |input, cx| {
            let title = format!("{name} · Development");
            input.set_value(
                if title.chars().count() <= ottero_core::MAX_TITLE_CHARS {
                    title
                } else {
                    name.to_owned()
                },
                window,
                cx,
            )
        });
        cx.notify();
    }

    pub fn start_new(&mut self, kind: ItemKind, window: &mut Window, cx: &mut Context<Self>) {
        let template = match kind {
            ItemKind::Login => Template::Login,
            ItemKind::Card => Template::Card,
            ItemKind::Note => Template::Note,
        };
        self.start_template(template, window, cx);
    }

    /// A new item from `template`, with its suggested fields ready to fill.
    pub fn start_template(
        &mut self,
        template: Template,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.id = None;
        self.item = None;
        self.mode = Mode::New(template.kind());
        self.template = Some(template);
        self.error = None;
        self.form.clear(window, cx);
        let fields = template.fields();
        self.environment_enabled = fields.environment.is_some();
        self.environment.update(cx, |editor, cx| {
            editor.fill(fields.environment.as_ref(), None, true, window, cx)
        });
        self.set_custom(&fields.custom, window, cx);
        self.form.title.update(cx, |i, cx| i.focus(window, cx));
        self.update_ticker(window, cx);
        cx.notify();
    }

    pub fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.item.clone().filter(|i| !i.is_trashed()) else {
            return;
        };
        self.form.fill(item.fields(), window, cx);
        self.environment_enabled = item.fields().environment.is_some();
        self.environment.update(cx, |editor, cx| {
            editor.fill(
                item.fields().environment.as_ref(),
                Some(item.id()),
                true,
                window,
                cx,
            )
        });
        self.set_custom(&item.fields().custom, window, cx);
        self.template = item.fields().template;
        self.mode = Mode::Edit;
        self.error = None;
        self.form.title.update(cx, |i, cx| i.focus(window, cx));
        self.update_ticker(window, cx);
        cx.notify();
    }

    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let env = self
            .item
            .as_ref()
            .and_then(|i| i.fields().environment.as_ref());
        self.environment_enabled = env.is_some();
        self.environment.update(cx, |editor, cx| {
            editor.fill(env, self.id, false, window, cx)
        });
        self.form.clear(window, cx);
        self.clear_custom(window, cx);
        self.mode = Mode::Read;
        self.error = None;
        self.update_ticker(window, cx);
        cx.notify();
    }

    /// Saves the draft. Stays in edit mode with a message if it's invalid.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = match self.mode {
            Mode::Read => return,
            Mode::Edit => self.item.as_ref().map_or(ItemKind::Login, VaultItem::kind),
            Mode::New(kind) => kind,
        };
        let mut fields = self.form.read(kind, cx);
        if self.environment_enabled {
            match self.environment.read(cx).document(cx) {
                Ok(document) => fields.environment = Some(document),
                Err(err) => {
                    self.error = Some(err.to_string().into());
                    cx.notify();
                    return;
                }
            }
        }
        fields.custom = self
            .custom
            .iter()
            .map(|r| CustomField {
                kind: r.kind,
                label: r.label.read(cx).value().to_string(),
                value: r.value.read(cx).value().to_string(),
            })
            .collect();
        fields.template = self.template;
        let result = match self.mode {
            Mode::New(_) => self
                .library
                .update(cx, |lib, cx| lib.create_vault_item(&fields, cx))
                .map(Some),
            _ => match self.id {
                Some(id) => self
                    .library
                    .update(cx, |lib, cx| {
                        lib.update_vault_item(id, cx, |item| {
                            item.update(&fields, ottero_core::clock::now())
                        })
                    })
                    .map(|()| None),
                None => Ok(None),
            },
        };
        ottero_crypto::Zeroize::zeroize(&mut fields);
        match result {
            Ok(created) => {
                self.form.clear(window, cx);
                self.clear_custom(window, cx);
                self.mode = Mode::Read;
                self.error = None;
                if let Some(id) = created {
                    self.id = Some(id);
                    cx.emit(PaneEvent::Created(id));
                }
                self.item = self.id.and_then(|id| self.library.read(cx).vault_item(id));
                let env = self
                    .item
                    .as_ref()
                    .and_then(|i| i.fields().environment.as_ref());
                self.environment_enabled = env.is_some();
                self.environment.update(cx, |editor, cx| {
                    editor.fill(env, self.id, false, window, cx)
                });
                self.update_ticker(window, cx);
            }
            Err(err) => self.error = Some(format!("{err:#}").into()),
        }
        cx.notify();
    }

    pub fn trash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.id else { return };
        let library = self.library.clone();
        let now = ottero_core::clock::now();
        let result = library.update(cx, |lib, cx| {
            lib.update_vault_item(id, cx, |i| i.trash(now).map(|()| true))
        });
        if feedback::report(result, window, cx).is_some() {
            cx.emit(PaneEvent::Removed);
            feedback::undo_toast("Moved to trash.", window, cx, move |window, cx| {
                notify_err(
                    library.update(cx, |lib, cx| {
                        lib.update_vault_item(id, cx, |i| {
                            i.restore(ottero_core::clock::now());
                            Ok(true)
                        })
                    }),
                    window,
                    cx,
                );
            });
        }
    }

    fn restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.id else { return };
        notify_err(
            self.library.update(cx, |lib, cx| {
                lib.update_vault_item(id, cx, |i| {
                    i.restore(ottero_core::clock::now());
                    Ok(true)
                })
            }),
            window,
            cx,
        );
    }

    fn delete_forever(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.item.clone() else {
            return;
        };
        let id = item.id();
        let library = self.library.clone();
        let pane = cx.entity().downgrade();
        feedback::confirm_destructive(
            Confirm::new(
                format!("Delete \u{201c}{}\u{201d} forever?", item.title()),
                "It's removed from this device. This can't be undone.",
                "Delete forever",
            ),
            window,
            cx,
            move |window, cx| {
                if feedback::report(
                    library.update(cx, |lib, cx| lib.delete_vault_items(&[id], cx)),
                    window,
                    cx,
                )
                .is_some()
                {
                    _ = pane.update(cx, |_, cx| cx.emit(PaneEvent::Removed));
                }
            },
        );
    }

    fn toggle_favorite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = &self.item else { return };
        let (id, fav) = (item.id(), !item.is_favorite());
        notify_err(
            self.library.update(cx, |lib, cx| {
                lib.update_vault_item(id, cx, |i| {
                    i.set_favorite(fav, ottero_core::clock::now())
                        .map(|()| true)
                })
            }),
            window,
            cx,
        );
    }

    fn duplicate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.id else { return };
        let result = self
            .library
            .update(cx, |lib, cx| lib.duplicate_vault_item(id, cx));
        if let Some(new_id) = feedback::report(result, window, cx) {
            cx.emit(PaneEvent::Selected(new_id));
        }
    }

    fn open_generator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.form.password.clone();
        crate::views::vault_generator::open_generator(Some(target), &self.library, window, cx);
    }

    // ---- Read mode --------------------------------------------------------

    /// One field row: label over value, actions on the right.
    #[allow(clippy::too_many_arguments)]
    fn field(
        &self,
        key: impl Into<SharedString>,
        label: impl Into<SharedString>,
        value: &str,
        secret: bool,
        mono: bool,
        extra: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let key: SharedString = key.into();
        let label: SharedString = label.into();
        let shown = !secret || self.revealed.contains(&key);
        let display: SharedString = if shown {
            if key.as_ref() == "number" {
                group_card(value)
            } else {
                value.to_owned()
            }
            .into()
        } else if key.as_ref() == "number" {
            let digits: String = value.chars().filter(char::is_ascii_digit).collect();
            format!(
                "•••• {}",
                digits
                    .get(digits.len().saturating_sub(4)..)
                    .unwrap_or_default()
            )
            .into()
        } else {
            "••••••••••".into()
        };
        // Codes show grouped ("123 456") but copy as typed.
        let copy_value = if key.as_ref() == "totp" || key.starts_with("code-") {
            value.replace(' ', "")
        } else {
            value.to_owned()
        };
        h_flex()
            .id(SharedString::from(format!("vault-field-{key}")))
            .gap_3()
            .px_4()
            .py_2p5()
            .items_center()
            .hover(|s| s.bg(theme.list_hover))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(label.clone()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .truncate()
                            .when(mono || secret, |s| {
                                s.font_family(theme.mono_font_family.clone())
                            })
                            .child(display),
                    ),
            )
            .children(extra)
            .when(secret, |s| {
                let key = key.clone();
                s.child(
                    Button::new(SharedString::from(format!("reveal-{key}")))
                        .ghost()
                        .xsmall()
                        .icon(if shown { Lucide::EyeOff } else { Lucide::Eye })
                        .tooltip(if shown { "Hide" } else { "Reveal" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.revealed.remove(&key) {
                                this.revealed.insert(key.clone());
                            }
                            cx.notify();
                        })),
                )
            })
            .child({
                let library = self.library.clone();
                let used = self.id;
                Button::new(SharedString::from(format!("copy-{key}")))
                    .ghost()
                    .xsmall()
                    .icon(Lucide::Copy)
                    .tooltip(format!("Copy {}", label.to_lowercase()))
                    .on_click(move |_, window, cx| {
                        copy_field(&label, copy_value.clone(), secret, window, cx);
                        if let Some(id) = used {
                            library.update(cx, |lib, cx| lib.mark_vault_used(id, cx));
                        }
                    })
            })
            .into_any_element()
    }

    fn totp_row(&self, item: &VaultItem, cx: &mut Context<Self>) -> Option<AnyElement> {
        let otp = totp::parse(&item.fields().totp).ok()?;
        let code = otp.code_at(unix_now());
        let ring = countdown(&code, cx);
        Some(self.field(
            "totp",
            "One-time code",
            &code.grouped(),
            false,
            true,
            Some(ring),
            cx,
        ))
    }

    fn read_view(&mut self, item: &VaultItem, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let f = item.fields().clone();
        let id = item.id();
        let trashed = item.is_trashed();
        let strength = self.library.update(cx, |lib, _| lib.vault_strength(id));
        let issues = self
            .library
            .update(cx, |lib, _| lib.vault_health().issues_for(id));
        let mut rows: Vec<AnyElement> = Vec::new();
        match f.kind {
            ItemKind::Login => {
                if !f.username.is_empty() {
                    rows.push(self.field(
                        "username",
                        "Username",
                        &f.username,
                        false,
                        false,
                        None,
                        cx,
                    ));
                }
                if !f.password.is_empty() {
                    let tag = strength.map(|s| {
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(s.label())
                            .into_any_element()
                    });
                    rows.push(self.field("password", "Password", &f.password, true, true, tag, cx));
                }
                if let Some(row) = self.totp_row(item, cx) {
                    rows.push(row);
                }
                for (i, url) in f.urls.iter().enumerate() {
                    rows.push(self.website_row(i, url, cx));
                }
            }
            ItemKind::Card => {
                for (key, label, value, secret) in [
                    ("cardholder", "Name on card", &f.cardholder, false),
                    ("number", "Number", &f.number, true),
                    ("expiry", "Expires", &f.expiry, false),
                    ("cvv", "Security code", &f.cvv, true),
                    ("pin", "PIN", &f.pin, true),
                ] {
                    if !value.is_empty() {
                        rows.push(self.field(
                            key,
                            label,
                            value,
                            secret,
                            key != "cardholder",
                            None,
                            cx,
                        ));
                    }
                }
            }
            ItemKind::Note => {}
        }
        for (i, c) in f.custom.iter().enumerate() {
            if let Some(row) = self.custom_row(i, c, cx) {
                rows.push(row);
            }
        }
        let n = rows.len();
        let rows: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                div()
                    .when(i + 1 < n, |s| s.border_b_1().border_color(theme.border))
                    .child(r)
                    .into_any_element()
            })
            .collect();

        let view = cx.entity().downgrade();
        let more = Button::new("vault-item-more")
            .ghost()
            .small()
            .icon(Lucide::Ellipsis)
            .tooltip("Item")
            .dropdown_menu(move |menu, _, _| {
                let (dup, trash, restore, delete) =
                    (view.clone(), view.clone(), view.clone(), view.clone());
                if trashed {
                    menu.item(
                        PopupMenuItem::new("Restore")
                            .icon(Lucide::ArchiveRestore)
                            .on_click(move |_, window, cx| {
                                _ = restore.update(cx, |this, cx| this.restore(window, cx));
                            }),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new("Delete forever…")
                            .icon(Lucide::Trash)
                            .on_click(move |_, window, cx| {
                                _ = delete.update(cx, |this, cx| this.delete_forever(window, cx));
                            }),
                    )
                } else {
                    menu.item(PopupMenuItem::new("Duplicate").icon(Lucide::Copy).on_click(
                        move |_, window, cx| {
                            _ = dup.update(cx, |this, cx| this.duplicate(window, cx));
                        },
                    ))
                    .separator()
                    .item(
                        PopupMenuItem::new("Move to trash")
                            .icon(Lucide::Trash)
                            .on_click(move |_, window, cx| {
                                _ = trash.update(cx, |this, cx| this.trash(window, cx));
                            }),
                    )
                }
            });

        let header = h_flex()
            .gap_3()
            .items_center()
            .child(avatar(item, rems(3.), cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(f.title.clone()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(match f.kind {
                                _ if f.environment.is_some() => "Project secrets".into(),
                                ItemKind::Login => f
                                    .urls
                                    .first()
                                    .map(|u| host(u))
                                    .unwrap_or_else(|| "Login".into()),
                                k => k.label().to_owned(),
                            }),
                    ),
            )
            .when(!trashed, |s| {
                s.child(
                    Button::new("vault-favorite")
                        .ghost()
                        .small()
                        .icon(Lucide::Star)
                        .selected(item.is_favorite())
                        .tooltip(if item.is_favorite() {
                            "Remove from favorites"
                        } else {
                            "Add to favorites"
                        })
                        .on_click(
                            cx.listener(|this, _, window, cx| this.toggle_favorite(window, cx)),
                        ),
                )
                .child(
                    Button::new("vault-edit")
                        .outline()
                        .small()
                        .label("Edit")
                        .on_click(cx.listener(|this, _, window, cx| this.edit(window, cx))),
                )
            })
            .child(more);

        let issue_line = (!issues.is_empty() && !trashed).then(|| {
            let text = issues
                .iter()
                .map(|i| match *i {
                    "Weak" => "This password is easy to guess.",
                    "Reused" => "This password is used by another item.",
                    _ => "This password hasn't changed in over a year.",
                })
                .collect::<Vec<_>>()
                .join(" ");
            h_flex()
                .gap_2()
                .px_3()
                .py_2()
                .rounded(theme.radius)
                .bg(theme.warning.opacity(0.1))
                .text_sm()
                .child(
                    Icon::new(Lucide::TriangleAlert)
                        .small()
                        .text_color(theme.warning),
                )
                .child(div().flex_1().child(text))
                .child(
                    Button::new("vault-fix")
                        .ghost()
                        .xsmall()
                        .label("Change…")
                        .on_click(cx.listener(|this, _, window, cx| this.edit(window, cx))),
                )
        });

        let history = item.history().to_vec();
        let history_open = self.history_open;
        let edited = item.updated_at().to_zoned(jiff::tz::TimeZone::system());
        let created = item.created_at().to_zoned(jiff::tz::TimeZone::system());

        v_flex()
            .gap_5()
            .when(trashed, |s| {
                s.child(
                    h_flex()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded(theme.radius)
                        .bg(theme.muted)
                        .text_sm()
                        .child(div().flex_1().child("This item is in the trash."))
                        .child(
                            Button::new("vault-restore")
                                .small()
                                .label("Restore")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.restore(window, cx)),
                                ),
                        ),
                )
            })
            .child(header)
            .children(issue_line)
            .when(!rows.is_empty(), |s| {
                s.child(
                    v_flex()
                        .rounded(theme.radius_lg)
                        .border_1()
                        .border_color(theme.border)
                        .overflow_hidden()
                        .children(rows),
                )
            })
            .when(f.environment.is_some(), |s| {
                s.child(self.environment.clone())
            })
            .when(!f.notes.is_empty(), |s| {
                s.child(
                    v_flex().gap_2().child(section_label("Notes", cx)).child(
                        div()
                            .p_4()
                            .rounded(theme.radius_lg)
                            .border_1()
                            .border_color(theme.border)
                            .text_sm()
                            .whitespace_normal()
                            .child(f.notes.clone()),
                    ),
                )
            })
            .when(!f.tags.is_empty(), |s| {
                s.child(
                    h_flex()
                        .gap_1()
                        .flex_wrap()
                        .children(f.tags.iter().map(|t| {
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded(theme.radius)
                                .bg(theme.muted)
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(format!("#{t}"))
                        })),
                )
            })
            .when(!history.is_empty(), |s| {
                s.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex().child(
                                Button::new("vault-history")
                                    .ghost()
                                    .xsmall()
                                    .icon(if history_open {
                                        Lucide::ChevronDown
                                    } else {
                                        Lucide::ChevronRight
                                    })
                                    .label(format!("Previous passwords ({})", history.len()))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.history_open = !this.history_open;
                                        cx.notify();
                                    })),
                            ),
                        )
                        .when(history_open, |s| {
                            s.child(
                                v_flex()
                                    .rounded(theme.radius_lg)
                                    .border_1()
                                    .border_color(theme.border)
                                    .children(history.iter().enumerate().map(|(i, h)| {
                                        let when = jiff::Timestamp::from_millisecond(h.replaced_at)
                                            .map(|t| {
                                                t.to_zoned(jiff::tz::TimeZone::system())
                                                    .strftime("%b %-d, %Y")
                                                    .to_string()
                                            })
                                            .unwrap_or_default();
                                        let value = h.password.clone();
                                        h_flex()
                                            .gap_3()
                                            .px_4()
                                            .py_2()
                                            .items_center()
                                            .when(i > 0, |s| {
                                                s.border_t_1().border_color(theme.border)
                                            })
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .text_sm()
                                                    .font_family(theme.mono_font_family.clone())
                                                    .child("••••••••••"),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme.muted_foreground)
                                                    .child(format!("Replaced {when}")),
                                            )
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "copy-old-{i}"
                                                )))
                                                .ghost()
                                                .xsmall()
                                                .icon(Lucide::Copy)
                                                .tooltip("Copy old password")
                                                .on_click(move |_, window, cx| {
                                                    copy_field(
                                                        "Old password",
                                                        value.clone(),
                                                        true,
                                                        window,
                                                        cx,
                                                    )
                                                }),
                                            )
                                    })),
                            )
                        }),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "Edited {} · Created {}",
                        edited.strftime("%b %-d, %Y"),
                        created.strftime("%b %-d, %Y")
                    )),
            )
            .into_any_element()
    }

    /// A custom field in read mode, shown by its kind.
    fn custom_row(&self, i: usize, c: &CustomField, cx: &mut Context<Self>) -> Option<AnyElement> {
        if c.value.is_empty() {
            return None;
        }
        let key = SharedString::from(format!("custom-{i}"));
        Some(match c.kind {
            FieldKind::Text | FieldKind::Date => {
                self.field(key, c.label.clone(), &c.value, false, false, None, cx)
            }
            FieldKind::Secret => self.field(key, c.label.clone(), &c.value, true, true, None, cx),
            FieldKind::Url => self.website_row(1000 + i, &c.value, cx),
            FieldKind::OneTimeCode => {
                let otp = totp::parse(&c.value).ok()?;
                let code = otp.code_at(unix_now());
                let ring = countdown(&code, cx);
                self.field(
                    SharedString::from(format!("code-{i}")),
                    c.label.clone(),
                    &code.grouped(),
                    false,
                    true,
                    Some(ring),
                    cx,
                )
            }
        })
    }

    /// Custom fields in edit mode: label, value, remove; then "Add field".
    fn custom_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let view = cx.entity().downgrade();
        let add = Button::new("custom-add")
            .ghost()
            .small()
            .icon(Lucide::Plus)
            .label("Add field")
            .dropdown_menu(move |menu, _, _| {
                FieldKind::ALL.into_iter().fold(menu, |menu, kind| {
                    let view = view.clone();
                    menu.item(
                        PopupMenuItem::new(kind.label()).on_click(move |_, window, cx| {
                            _ = view.update(cx, |this, cx| {
                                this.add_custom(kind, "", "", window, cx);
                                if let Some(row) = this.custom.last() {
                                    row.label.update(cx, |i, cx| i.focus(window, cx));
                                }
                            });
                        }),
                    )
                })
            });
        let rows = self.custom.iter().enumerate().map(|(i, row)| {
            let value = Input::new(&row.value).small();
            let value = if row.kind.is_secret() {
                value.mask_toggle()
            } else {
                value
            };
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .w(rems(9.))
                        .flex_shrink_0()
                        .child(Input::new(&row.label).small()),
                )
                .child(div().flex_1().min_w_0().child(value))
                .child(
                    div()
                        .w(rems(5.5))
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(row.kind.label()),
                )
                .child(
                    Button::new(SharedString::from(format!("custom-remove-{i}")))
                        .ghost()
                        .xsmall()
                        .icon(Lucide::X)
                        .tooltip("Remove field")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if i < this.custom.len() {
                                let row = this.custom.remove(i);
                                for input in [&row.label, &row.value] {
                                    input.update(cx, |inp, cx| inp.set_value("", window, cx));
                                }
                                cx.notify();
                            }
                        })),
                )
        });
        v_flex()
            .gap_2()
            .when(!self.custom.is_empty(), |s| {
                s.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("More fields"),
                )
            })
            .children(rows)
            .child(h_flex().child(add))
            .into_any_element()
    }

    fn website_row(&self, i: usize, url: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let open = openable_url(url);
        let copy_value = url.to_owned();
        h_flex()
            .id(SharedString::from(format!("vault-url-{i}")))
            .gap_3()
            .px_4()
            .py_2p5()
            .items_center()
            .hover(|s| s.bg(theme.list_hover))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Website"),
                    )
                    .child(div().text_sm().truncate().child(host(url))),
            )
            .children(open.map(|href| {
                Button::new(SharedString::from(format!("open-url-{i}")))
                    .ghost()
                    .xsmall()
                    .icon(Lucide::ExternalLink)
                    .tooltip("Open in browser")
                    .on_click(move |_, _, cx| cx.open_url(&href))
            }))
            .child(
                Button::new(SharedString::from(format!("copy-url-{i}")))
                    .ghost()
                    .xsmall()
                    .icon(Lucide::Copy)
                    .tooltip("Copy website")
                    .on_click(move |_, window, cx| {
                        copy_field("Website", copy_value.clone(), false, window, cx)
                    }),
            )
            .into_any_element()
    }

    // ---- Edit mode --------------------------------------------------------

    fn form_row(label: &'static str, control: AnyElement, cx: &App) -> AnyElement {
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(control)
            .into_any_element()
    }

    fn edit_view(&mut self, kind: ItemKind, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let f = &self.form;
        let mut rows: Vec<AnyElement> = Vec::new();
        match kind {
            ItemKind::Login => {
                rows.push(Self::form_row(
                    "Username",
                    Input::new(&f.username)
                        .id("edit-username")
                        .into_any_element(),
                    cx,
                ));
                let pw = f.password.read(cx).value().to_string();
                let meter = (!pw.is_empty()).then(|| {
                    let e = estimate(&pw, &[]);
                    strength_meter(e.strength, Some(&e.crack_time), cx).into_any_element()
                });
                rows.push(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("Password"),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .child(div().flex_1().child(
                                    Input::new(&f.password).id("edit-password").mask_toggle(),
                                ))
                                .child(
                                    Button::new("edit-generate")
                                        .outline()
                                        .small()
                                        .icon(Lucide::Dices)
                                        .tooltip("Generate a password")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_generator(window, cx)
                                        })),
                                ),
                        )
                        .children(meter)
                        .into_any_element(),
                );
                rows.push(Self::form_row(
                    "Websites",
                    Textarea::new(&f.urls).into_any_element(),
                    cx,
                ));
                rows.push(Self::form_row(
                    "One-time code",
                    Input::new(&f.totp)
                        .id("edit-totp")
                        .mask_toggle()
                        .into_any_element(),
                    cx,
                ));
            }
            ItemKind::Card => {
                rows.push(Self::form_row(
                    "Name on card",
                    Input::new(&f.cardholder).into_any_element(),
                    cx,
                ));
                rows.push(Self::form_row(
                    "Number",
                    Input::new(&f.number)
                        .id("edit-number")
                        .mask_toggle()
                        .into_any_element(),
                    cx,
                ));
                rows.push(
                    h_flex()
                        .gap_3()
                        .child(div().flex_1().child(Self::form_row(
                            "Expires",
                            Input::new(&f.expiry).into_any_element(),
                            cx,
                        )))
                        .child(div().flex_1().child(Self::form_row(
                            "Security code",
                            Input::new(&f.cvv).mask_toggle().into_any_element(),
                            cx,
                        )))
                        .child(div().flex_1().child(Self::form_row(
                            "PIN",
                            Input::new(&f.pin).mask_toggle().into_any_element(),
                            cx,
                        )))
                        .into_any_element(),
                );
            }
            ItemKind::Note => {}
        }
        if self.environment_enabled {
            rows.push(self.environment.clone().into_any_element());
        }
        rows.push(self.custom_editor(cx));
        rows.push(Self::form_row(
            if kind == ItemKind::Note {
                "Note"
            } else {
                "Notes"
            },
            Textarea::new(&f.notes).into_any_element(),
            cx,
        ));
        rows.push(Self::form_row(
            "Tags",
            Input::new(&f.tags).into_any_element(),
            cx,
        ));

        let heading = match self.mode {
            Mode::New(k) => format!(
                "New {}",
                self.template
                    .map_or(k.label(), Template::label)
                    .to_lowercase()
            ),
            _ => "Edit".to_owned(),
        };
        v_flex()
            .gap_5()
            .on_key_down(cx.listener(|this, e: &gpui_kit::KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.cancel(window, cx);
                }
            }))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(heading),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("edit-cancel")
                                    .ghost()
                                    .small()
                                    .label("Cancel")
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.cancel(window, cx)),
                                    ),
                            )
                            .child(
                                Button::new("edit-save")
                                    .primary()
                                    .small()
                                    .label("Save")
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.save(window, cx)),
                                    ),
                            ),
                    ),
            )
            .child(
                Input::new(&f.title)
                    .id("edit-title")
                    .large()
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .children(
                self.error
                    .clone()
                    .map(|e| div().text_sm().text_color(theme.danger).child(e)),
            )
            .child(v_flex().gap_4().children(rows))
            .into_any_element()
    }

    fn empty(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let lib = self.library.read(cx);
        let total = lib.vault().count(crate::vault::VaultFilter::All);
        if total == 0 {
            return empty_state(
                Lucide::KeyRound,
                "Your vault is empty",
                "Add a login with New, or import from Bitwarden, 1Password or your browser (… → Import from CSV).",
                cx,
            )
            .into_any_element();
        }
        let report = self.library.update(cx, |lib, _| lib.vault_health());
        let theme = cx.theme().clone();
        let tile = |label: &'static str, n: usize, cx: &App| {
            v_flex()
                .flex_1()
                .min_w(rems(7.))
                .gap_1()
                .p_3()
                .rounded(theme.radius_lg)
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(label),
                )
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(n.to_string()),
                )
        };
        let reused: usize = report.reused.iter().map(Vec::len).sum();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .p_8()
            .child(
                v_flex()
                    .w_full()
                    .max_w(rems(30.))
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().text_color(theme.muted_foreground).child("Password health"))
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("{}%", report.score())),
                            )
                            .child(
                                div().text_sm().text_color(theme.muted_foreground).child(format!(
                                    "{} of {} logins have no issues. Checked on this device; nothing is sent anywhere.",
                                    report.clean(),
                                    report.checked
                                )),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .flex_wrap()
                            .child(tile("Weak", report.weak.len(), cx))
                            .child(tile("Reused", reused, cx))
                            .child(tile("Over a year old", report.old.len(), cx)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("Select an item to see it, or open Password health in the sidebar for the list."),
                    ),
            )
            .into_any_element()
    }
}

/// "4111 1111 1111 1111" from any spacing.
/// The ring and seconds left for a one-time code.
pub fn countdown(code: &totp::Code, cx: &App) -> AnyElement {
    let left = code.seconds_left as f32 / code.period as f32;
    h_flex()
        .gap_1p5()
        .items_center()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(
            ProgressCircle::new("totp-ring")
                .value(left * 100.)
                .small()
                .accessibility_label("Time left for this code"),
        )
        .child(format!("{}s", code.seconds_left))
        .into_any_element()
}

fn group_card(number: &str) -> String {
    let digits: Vec<char> = number.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 12 {
        return number.to_owned();
    }
    digits
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Render for VaultItemPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match (self.mode, self.item.clone()) {
            (Mode::New(kind), _) => self.edit_view(kind, cx),
            (Mode::Edit, Some(item)) => self.edit_view(item.kind(), cx),
            (_, Some(item)) => self.read_view(&item, cx),
            (_, None) => return div().size_full().child(self.empty(cx)).into_any_element(),
        };
        div()
            .id("vault-pane")
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .w_full()
                    .max_w(rems(40.))
                    .mx_auto()
                    .px_8()
                    .py_8()
                    .child(body),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_numbers_group_in_fours() {
        assert_eq!(group_card("4111111111111111"), "4111 1111 1111 1111");
        assert_eq!(group_card("123"), "123");
    }
}
