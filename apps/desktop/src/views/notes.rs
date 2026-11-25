//! Notes screen: searchable list on the left, the note on the right.
//!
//! The body is a live-preview Markdown editor ([`NoteEditor`]); trashed notes
//! show read-only rendered Markdown.
//!
//! Editor state machine:
//!
//! ```text
//!  Empty ──open(id)──▶ Clean ──typing──▶ Dirty ──500 ms idle / switch / blur──▶ Clean
//!                        ▲                 │
//!                        └──── save ok ────┘   (save error: stays Dirty, user notified)
//! ```
//!
//! Content is always flushed before another note opens, so switching notes
//! can never lose typing.

use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    TestSupportExt as _, Window, div, px, rems,
};
use ottero_core::NoteId;
use ottero_core::links::{note_link_url, parse_note_link_url};
use ottero_storage::NoteSummary;
use ottero_ui::appearance;
use ottero_ui::components::{empty_state, section_label};
use ottero_ui::feedback::{self, Confirm};
use ottero_ui::panel::{PanelSide, SidePanel};

use crate::actions::{
    FormatBold, FormatBullets, FormatChecklist, FormatCode, FormatHeading1, FormatHeading2,
    FormatHeading3, FormatHighlight, FormatItalic, FormatLink, FormatNumbers, FormatParagraph,
    FormatQuote, FormatStrikethrough,
};
use crate::library::{Library, LibraryEvent};
use crate::ui_state::{LayoutStore, NOTES_LIST};
use crate::views::Navigate;
use crate::views::note_editor::{NoteEditor, NoteEditorEvent};
use crate::views::task_row::notify_err;

const AUTOSAVE_DELAY: Duration = Duration::from_millis(500);
const SEARCH_LIMIT: usize = 200;

/// The note currently loaded in the editor.
#[derive(Clone)]
struct OpenNote {
    id: NoteId,
    title: String,
    pinned: bool,
    trashed: bool,
}

pub struct NotesView {
    library: Entity<Library>,
    layout: Entity<LayoutStore>,
    search: Entity<InputState>,
    title: Entity<InputState>,
    body: Entity<NoteEditor>,
    list: Vec<NoteSummary>,
    load_error: bool,
    show_trash: bool,
    /// Shows only notes with this tag (lower-case, no `#`).
    tag_filter: Option<String>,
    mentions: Vec<NoteSummary>,
    /// The note page's single scroll area (the editor grows with its text).
    page_scroll: ScrollHandle,
    open: Option<OpenNote>,
    backlinks: Vec<NoteSummary>,
    dirty: bool,
    /// Suppresses `schedule_save` while loading or live-refreshing a note.
    reloading: bool,
    pending_save: Option<Task<()>>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Navigate> for NotesView {}

impl NotesView {
    pub fn new(
        library: Entity<Library>,
        layout: Entity<LayoutStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search notes")
                .clean_on_escape()
        });
        let title = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Untitled")
                // Rejects keystrokes past the domain limit, so a save can never fail on length.
                .validate(|text, _| text.chars().count() <= ottero_core::MAX_TITLE_CHARS)
        });
        let body = cx.new(|cx| NoteEditor::new(library.downgrade(), window, cx));

        let subscriptions = vec![
            cx.observe(&layout, |_, _, cx| cx.notify()),
            cx.subscribe_in(&library, window, |this, _, event, window, cx| {
                if *event == LibraryEvent::NotesChanged {
                    this.reload_list(cx);
                    this.refresh_open_note(window, cx);
                }
            }),
            cx.subscribe(&search, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.reload_list(cx);
                }
            }),
            cx.subscribe_in(&title, window, |this, _, event, window, cx| match event {
                InputEvent::Change => this.schedule_save(window, cx),
                InputEvent::PressEnter { .. } => this.focus_body(window, cx),
                InputEvent::Blur => {
                    _ = this.flush(window, cx);
                }
                _ => {}
            }),
            cx.subscribe_in(&body, window, |this, _, event, window, cx| match event {
                NoteEditorEvent::Changed => this.schedule_save(window, cx),
                // Layout is known after the next paint; scroll then.
                NoteEditorEvent::CaretMoved => cx.on_next_frame(window, |_, window, cx| {
                    cx.on_next_frame(window, |this, _, cx| this.follow_caret(cx));
                }),
                NoteEditorEvent::Blurred => {
                    _ = this.flush(window, cx);
                }
                NoteEditorEvent::OpenNote(target) => this.follow_link(target, window, cx),
                NoteEditorEvent::OpenTag(tag) => {
                    this.set_tag_filter(Some(tag.clone()), window, cx);
                }
                NoteEditorEvent::FindMoved => cx.on_next_frame(window, |_, window, cx| {
                    cx.on_next_frame(window, |this, _, cx| this.reveal_match(cx));
                }),
            }),
        ];

        let mut this = Self {
            layout,
            library,
            search,
            title,
            body,
            list: Vec::new(),
            load_error: false,
            show_trash: false,
            tag_filter: None,
            mentions: Vec::new(),
            page_scroll: ScrollHandle::new(),
            open: None,
            backlinks: Vec::new(),
            dirty: false,
            reloading: false,
            pending_save: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.reload_list(cx);
        this
    }

    /// Creates a note and opens it with the title focused.
    pub fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush(window, cx) {
            return;
        }
        self.show_trash = false;
        match self.library.update(cx, |lib, cx| lib.create_note("", cx)) {
            Ok(id) => {
                self.open_note(id, window, cx);
                self.title.update(cx, |t, cx| t.focus(window, cx));
            }
            Err(err) => notify_err::<()>(Err(err), window, cx),
        }
    }

    /// Opens `id`, saving the current note first.
    pub fn open_note(&mut self, id: NoteId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush(window, cx) {
            return;
        }
        let note = match self.library.read(cx).note(id) {
            Ok(Some(note)) => note,
            Ok(None) => {
                notify_err::<()>(
                    Err(anyhow::anyhow!("That note no longer exists")),
                    window,
                    cx,
                );
                return;
            }
            Err(err) => return notify_err::<()>(Err(err), window, cx),
        };
        if note.is_trashed() != self.show_trash {
            self.show_trash = note.is_trashed();
            self.reload_list(cx);
        }
        // An untitled note shows an empty title field with the placeholder.
        let title_text = if note.title() == ottero_core::note::UNTITLED {
            ""
        } else {
            note.title()
        };
        self.title
            .update(cx, |t, cx| t.set_value(title_text.to_owned(), window, cx));
        self.body
            .update(cx, |b, cx| b.set_note(Some(id), note.body(), cx));
        self.open = Some(OpenNote {
            id,
            title: note.title().to_owned(),
            pinned: note.is_pinned(),
            trashed: note.is_trashed(),
        });
        self.dirty = false;
        self.page_scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
        self.reload_backlinks(cx);
        cx.notify();
    }

    /// Opens the first listed note if nothing is open (called when the screen is shown).
    pub fn ensure_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.is_none()
            && !self.show_trash
            && let Some(first) = self.list.first().map(|n| n.id)
        {
            self.open_note(first, window, cx);
        }
    }

    /// Opens the note titled `title` and puts the caret at the end (dev and perf scripts).
    pub fn open_note_titled_at_end(
        &mut self,
        title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Ok(Some(id)) = self.library.read(cx).find_note_by_title(title) {
            self.open_note(id, window, cx);
            self.body.update(cx, |b, cx| b.move_to_end(window, cx));
        }
    }

    pub fn focus_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.body.update(cx, |b, cx| b.focus(window, cx));
    }

    /// The body editor (tests inspect its link menu and selection bar).
    pub fn note_editor(&self) -> &Entity<NoteEditor> {
        &self.body
    }

    /// Title of the open note, if any.
    pub fn open_note_title(&self, _: &App) -> Option<String> {
        self.open.as_ref().map(|o| o.title.clone())
    }

    /// The open note, if any.
    pub fn open_note_id(&self) -> Option<NoteId> {
        self.open.as_ref().map(|o| o.id)
    }

    /// Current editor body text (for tests and diagnostics).
    pub fn body_text(&self, cx: &App) -> String {
        self.body.read(cx).text(cx)
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.focus(window, cx));
    }

    fn reload_list(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().to_string();
        let library = self.library.read(cx);
        let result = if self.show_trash {
            library.notes(true)
        } else {
            let base = if query.trim().is_empty() {
                library.notes(false)
            } else {
                library.search_notes(&query, SEARCH_LIMIT)
            };
            match &self.tag_filter {
                None => base,
                Some(tag) => {
                    let tagged = library.note_ids_with_tag(tag);
                    base.map(|list| {
                        list.into_iter()
                            .filter(|n| tagged.contains(&n.id))
                            .collect()
                    })
                }
            }
        };
        match result {
            Ok(list) => {
                self.list = list;
                self.load_error = false;
            }
            Err(err) => {
                eprintln!("ottero: failed to load notes: {err:#}");
                self.list.clear();
                self.load_error = true;
            }
        }
        cx.notify();
    }

    /// Shows only notes tagged `tag` (or all notes for `None`).
    pub fn set_tag_filter(
        &mut self,
        tag: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tag = tag
            .map(|t| ottero_core::tags::normalize_tag(&t))
            .filter(|t| !t.is_empty());
        if tag == self.tag_filter && !self.show_trash {
            return;
        }
        if self.show_trash {
            self.set_show_trash(false, window, cx);
        }
        self.tag_filter = tag;
        self.reload_list(cx);
    }

    pub fn tag_filter(&self) -> Option<&str> {
        self.tag_filter.as_deref()
    }

    /// "Aa": the six everyday styles first, then Headings ▸ and Lists ▸.
    /// Items dispatch the same actions as the shortcuts, into the editor, so the
    /// menu also teaches the keys. (Selecting text shows the same inline styles
    /// in a small bar above it.)
    fn format_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let editor = self.body.read(cx).focus_handle(cx);
        Button::new("note-format")
            .ghost()
            .small()
            .icon(Lucide::Type)
            .tooltip("Format")
            .dropdown_menu(move |menu, window, cx| {
                menu.action_context(editor.clone())
                    .menu_with_icon("Bold", Lucide::Bold, Box::new(FormatBold))
                    .menu_with_icon("Italic", Lucide::Italic, Box::new(FormatItalic))
                    .menu_with_icon(
                        "Strikethrough",
                        Lucide::Strikethrough,
                        Box::new(FormatStrikethrough),
                    )
                    .menu_with_icon("Highlight", Lucide::Highlighter, Box::new(FormatHighlight))
                    .menu_with_icon("Code", Lucide::Code, Box::new(FormatCode))
                    .menu_with_icon("Link", Lucide::Link2, Box::new(FormatLink))
                    .separator()
                    .submenu_with_icon(
                        Some(Lucide::Heading.into()),
                        "Headings",
                        window,
                        cx,
                        |sub, _, _| {
                            sub.menu_with_icon(
                                "Heading 1",
                                Lucide::Heading1,
                                Box::new(FormatHeading1),
                            )
                            .menu_with_icon("Heading 2", Lucide::Heading2, Box::new(FormatHeading2))
                            .menu_with_icon("Heading 3", Lucide::Heading3, Box::new(FormatHeading3))
                            .separator()
                            .menu_with_icon(
                                "Body text",
                                Lucide::Pilcrow,
                                Box::new(FormatParagraph),
                            )
                        },
                    )
                    .submenu_with_icon(
                        Some(Lucide::List.into()),
                        "Lists",
                        window,
                        cx,
                        |sub, _, _| {
                            sub.menu_with_icon(
                                "Bulleted list",
                                Lucide::List,
                                Box::new(FormatBullets),
                            )
                            .menu_with_icon(
                                "Numbered list",
                                Lucide::ListOrdered,
                                Box::new(FormatNumbers),
                            )
                            .menu_with_icon(
                                "Checklist",
                                Lucide::ListChecks,
                                Box::new(FormatChecklist),
                            )
                            .separator()
                            .menu_with_icon(
                                "Quote",
                                Lucide::Quote,
                                Box::new(FormatQuote),
                            )
                        },
                    )
            })
    }

    /// "…" for the open note: pin, view, export, trash (open → change → destructive).
    fn note_menu(&self, pinned: bool, cx: &Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let source = self.body.read(cx).is_source_mode();
        Button::new("note-more")
            .ghost()
            .small()
            .icon(Lucide::Ellipsis)
            .tooltip("More")
            .dropdown_menu(move |menu, _, _| {
                let run = |label: &'static str, icon: Lucide, f: fn(&mut NotesView, &mut Window, &mut Context<NotesView>)| {
                    let view = view.clone();
                    PopupMenuItem::new(label).icon(icon).on_click(move |_, window, cx| {
                        _ = view.update(cx, |this, cx| f(this, window, cx));
                    })
                };
                menu.item(if pinned {
                    run("Unpin", Lucide::PinOff, |this, w, cx| this.toggle_pin_open(w, cx))
                } else {
                    run("Pin to top", Lucide::Pin, |this, w, cx| this.toggle_pin_open(w, cx))
                })
                .item(
                    run("Show Markdown source", Lucide::Code, |this, w, cx| this.toggle_source_mode(w, cx))
                        .checked(source),
                )
                .item(run("Export all as Markdown…", Lucide::FolderOutput, |this, w, cx| this.export_notes(w, cx)))
                .separator()
                .item(run("Move to trash", Lucide::Trash, |this, w, cx| {
                    if let Some(id) = this.open.as_ref().map(|o| o.id) {
                        this.trash_note(id, w, cx);
                    }
                }))
            })
    }

    fn toggle_pin_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((id, pinned)) = self.open.as_ref().map(|o| (o.id, o.pinned)) {
            self.set_pinned(id, !pinned, window, cx);
        }
    }

    /// Opens the find bar for the open note (⌘F).
    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.as_ref().is_none_or(|o| o.trashed) {
            return;
        }
        self.body.update(cx, |b, cx| b.open_find(window, cx));
        cx.notify();
    }

    /// Next (`1`) or previous (`-1`) match (⌘G / ⇧⌘G). Opens the bar if closed.
    pub fn step_find(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.body.read(cx).find().is_none() {
            return self.open_find(window, cx);
        }
        self.body.update(cx, |b, cx| b.step_find(delta, cx));
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.body.update(cx, |b, cx| b.close_find(window, cx));
        cx.notify();
    }

    /// Scrolls so the active find match sits in the upper third of the page.
    fn reveal_match(&mut self, cx: &mut Context<Self>) {
        let Some(top) = self.body.read(cx).active_match_top(cx) else {
            return;
        };
        let viewport = self.page_scroll.bounds();
        if viewport.size.height <= px(0.) {
            return;
        }
        let line = cx.theme().font_size * 2.;
        if top >= viewport.top() + line && top + line <= viewport.bottom() - line {
            return; // already comfortably visible
        }
        let mut offset = self.page_scroll.offset();
        offset.y += viewport.top() + viewport.size.height / 3. - top;
        let max = self.page_scroll.max_offset().y;
        offset.y = offset.y.min(px(0.)).max(-max);
        self.page_scroll.set_offset(offset);
        cx.notify();
    }

    /// The find bar, in the toolbar's left slot: field, "2 of 7", ↑ ↓, close.
    fn render_find_bar(&self, cx: &Context<Self>) -> Option<gpui_kit::AnyElement> {
        let body = self.body.read(cx);
        let find = body.find()?;
        let (at, total) = body.find_status().unwrap_or((0, 0));
        let query_empty = find.input.read(cx).value().is_empty();
        let theme = cx.theme();
        let status: SharedString = match (query_empty, total) {
            (true, _) => "".into(),
            (false, 0) => "No matches".into(),
            (false, n) => format!("{at} of {n}").into(),
        };
        let nav = |id: &'static str,
                   icon: Lucide,
                   label: &'static str,
                   action: Box<dyn gpui_kit::Action>,
                   delta: isize,
                   cx: &Context<Self>| {
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(icon)
                .disabled(total == 0)
                .tooltip_with_action(label, action.as_ref(), None)
                .on_click(cx.listener(move |this, _, window, cx| this.step_find(delta, window, cx)))
        };
        Some(
            h_flex()
                .id("find-bar")
                .test_support()
                .gap_1()
                .items_center()
                .w(rems(22.))
                .key_context("FindBar")
                .on_action(cx.listener(
                    |this, _: &gpui_kit::component::input::Escape, window, cx| {
                        this.close_find(window, cx)
                    },
                ))
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(&find.input).small().prefix(
                            Icon::new(Lucide::Search)
                                .xsmall()
                                .text_color(theme.muted_foreground),
                        ),
                    ),
                )
                .child(
                    div()
                        .id("find-status")
                        .min_w(rems(4.5))
                        .text_xs()
                        .text_color(if total == 0 && !query_empty {
                            theme.danger
                        } else {
                            theme.muted_foreground
                        })
                        .child(status),
                )
                .child(nav(
                    "find-prev",
                    Lucide::ChevronUp,
                    "Previous match",
                    Box::new(crate::actions::FindPrevious),
                    -1,
                    cx,
                ))
                .child(nav(
                    "find-next",
                    Lucide::ChevronDown,
                    "Next match",
                    Box::new(crate::actions::FindNext),
                    1,
                    cx,
                ))
                .child(
                    Button::new("find-close")
                        .ghost()
                        .xsmall()
                        .icon(Lucide::X)
                        .tooltip("Close (Esc)")
                        .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
                )
                .into_any_element(),
        )
    }

    /// Shows the raw Markdown instead of the live preview (and back).
    pub fn toggle_source_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.as_ref().is_none_or(|o| o.trashed) {
            return;
        }
        self.body.update(cx, |b, cx| {
            let on = !b.is_source_mode();
            b.set_source_mode(on, window, cx);
            b.focus(window, cx);
        });
        cx.notify();
    }

    pub fn is_source_mode(&self, cx: &App) -> bool {
        self.body.read(cx).is_source_mode()
    }

    /// Opens (creating if needed) today's daily note.
    pub fn open_daily_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush(window, cx) {
            return;
        }
        let today = ottero_core::clock::today();
        let result = self
            .library
            .update(cx, |lib, cx| lib.open_daily_note(today, cx));
        if let Some(id) = feedback::report(result, window, cx) {
            self.show_trash = false;
            self.tag_filter = None;
            self.reload_list(cx);
            self.open_note(id, window, cx);
            self.focus_body(window, cx);
        }
    }

    fn link_mention(&mut self, source: NoteId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.open.as_ref().map(|o| o.title.clone()) else {
            return;
        };
        let result = self
            .library
            .update(cx, |lib, cx| lib.link_mention(source, &title, cx));
        if feedback::report(result, window, cx).is_some() {
            self.reload_backlinks(cx);
            cx.notify();
        }
    }

    fn reload_backlinks(&mut self, cx: &mut Context<Self>) {
        self.mentions = match &self.open {
            Some(open) if !open.trashed => self
                .library
                .read(cx)
                .unlinked_mentions(open.id, &open.title),
            _ => Vec::new(),
        };
        self.backlinks = match &self.open {
            Some(open) if !open.trashed => self
                .library
                .read(cx)
                .backlinks(&open.title)
                .unwrap_or_default()
                .into_iter()
                .filter(|b| b.id != open.id)
                .collect(),
            _ => Vec::new(),
        };
    }

    /// Live-refreshes the currently open note when a remote sync (or external
    /// mutation) updates SQLite while the editor is clean. Preserves scroll
    /// position and avoids triggering an echo-save loop.
    fn refresh_open_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dirty {
            return;
        }
        let Some(open) = self.open.clone() else {
            return;
        };
        let note = match self.library.read(cx).note(open.id) {
            Ok(Some(note)) => note,
            Ok(None) => {
                self.open = None;
                self.pending_save = None;
                self.backlinks.clear();
                self.mentions.clear();
                cx.notify();
                return;
            }
            Err(_) => return,
        };
        let title_text = if note.title() == ottero_core::note::UNTITLED {
            ""
        } else {
            note.title()
        };
        let title_changed = self.title.read(cx).value().as_ref() != title_text;
        let body_changed = self.body.read(cx).text(cx) != note.body();
        let meta_changed = open.title != note.title()
            || open.pinned != note.is_pinned()
            || open.trashed != note.is_trashed();
        if !title_changed && !body_changed && !meta_changed {
            self.reload_backlinks(cx);
            return;
        }
        self.reloading = true;
        if title_changed {
            self.title
                .update(cx, |t, cx| t.set_value(title_text.to_owned(), window, cx));
        }
        if body_changed {
            self.body
                .update(cx, |b, cx| b.set_note(Some(open.id), note.body(), cx));
        }
        self.open = Some(OpenNote {
            id: open.id,
            title: note.title().to_owned(),
            pinned: note.is_pinned(),
            trashed: note.is_trashed(),
        });
        self.dirty = false;
        self.reloading = false;
        self.reload_backlinks(cx);
        cx.notify();
    }

    fn schedule_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reloading || self.open.as_ref().is_none_or(|o| o.trashed) {
            return;
        }
        self.dirty = true;
        // Replacing the task cancels the previous timer: a debounce.
        self.pending_save = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(AUTOSAVE_DELAY).await;
            _ = this.update_in(cx, |this, window, cx| this.flush(window, cx));
        }));
    }

    /// Writes pending edits now. Returns `false` if saving failed: the editor
    /// stays Dirty and callers must not replace its content (that would lose typing).
    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.save_pending(cx) {
            Ok(()) => true,
            Err(err) => {
                notify_err::<()>(Err(err), window, cx);
                false
            }
        }
    }

    /// Saves pending edits without UI feedback (used on quit and window close).
    pub fn save_pending(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        self.pending_save = None;
        if !self.dirty {
            return Ok(());
        }
        let Some(open) = &self.open else {
            self.dirty = false;
            return Ok(());
        };
        let id = open.id;
        let title = self.title.read(cx).value().to_string();
        let body = self.body.read(cx).text(cx);
        self.library
            .update(cx, |lib, cx| lib.edit_note(id, &title, &body, cx))?;
        self.dirty = false;
        let saved_title = ottero_core::normalize_title(&title)
            .unwrap_or_else(|_| ottero_core::note::UNTITLED.to_owned());
        if let Some(open) = &mut self.open
            && open.title != saved_title
        {
            open.title = saved_title;
            self.reload_backlinks(cx);
        }
        Ok(())
    }

    fn set_pinned(
        &mut self,
        id: NoteId,
        pinned: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self
            .library
            .update(cx, |lib, cx| lib.set_note_pinned(id, pinned, cx));
        if feedback::report(result, window, cx).is_some()
            && let Some(open) = self.open.as_mut().filter(|o| o.id == id)
        {
            open.pinned = pinned;
        }
    }

    fn restore_note(&mut self, id: NoteId, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.library.update(cx, |lib, cx| lib.restore_note(id, cx));
        if feedback::report(result, window, cx).is_some() {
            self.show_trash = false;
            self.reload_list(cx);
            self.open_note(id, window, cx);
        }
    }

    /// Moves any note to the trash (the open one or one from the list) and offers Undo.
    fn trash_note(&mut self, id: NoteId, window: &mut Window, cx: &mut Context<Self>) {
        let is_open = self.open.as_ref().is_some_and(|o| o.id == id);
        if is_open && !self.flush(window, cx) {
            return;
        }
        let result = self.library.update(cx, |lib, cx| lib.trash_note(id, cx));
        if feedback::report(result, window, cx).is_none() {
            return;
        }
        if is_open {
            self.open = None;
        }
        let library = self.library.clone();
        let view = cx.entity().downgrade();
        feedback::undo_toast("Note moved to trash.", window, cx, move |window, cx| {
            if feedback::report(
                library.update(cx, |lib, cx| lib.restore_note(id, cx)),
                window,
                cx,
            )
            .is_some()
            {
                _ = view.update(cx, |this, cx| this.open_note(id, window, cx));
            }
        });
        cx.notify();
    }

    fn restore_open(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.open.as_ref().map(|o| o.id) {
            self.restore_note(id, window, cx);
        }
    }

    fn delete_open(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_ref() else {
            return;
        };
        let id = open.id;
        let library = self.library.clone();
        let view = cx.entity().downgrade();
        feedback::confirm_destructive(
            Confirm::new(
                format!("Delete \u{201c}{}\u{201d} forever?", open.title),
                "The note and its links will be removed. This can't be undone.",
                "Delete",
            ),
            window,
            cx,
            move |window, cx| {
                if feedback::report(
                    library.update(cx, |lib, cx| lib.delete_note_permanently(id, cx)),
                    window,
                    cx,
                )
                .is_some()
                {
                    _ = view.update(cx, |this, cx| {
                        this.open = None;
                        cx.notify();
                    });
                }
            },
        );
    }

    fn empty_trash(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let library = self.library.clone();
        let view = cx.entity().downgrade();
        feedback::confirm_destructive(
            Confirm::new(
                "Empty note trash?",
                format!(
                    "{} note(s) will be deleted forever. This can't be undone.",
                    self.list.len()
                ),
                "Empty trash",
            ),
            window,
            cx,
            move |window, cx| {
                if feedback::report(
                    library.update(cx, |lib, cx| lib.empty_note_trash(cx)),
                    window,
                    cx,
                )
                .is_some()
                {
                    _ = view.update(cx, |this, cx| {
                        this.open = None;
                        cx.notify();
                    });
                }
            },
        );
    }

    pub fn is_showing_trash(&self) -> bool {
        self.show_trash
    }

    pub fn set_show_trash(&mut self, show: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_trash == show {
            return;
        }
        if !self.flush(window, cx) {
            return;
        }
        self.show_trash = show;
        self.open = None;
        self.reload_list(cx);
    }

    fn render_list(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let selected = self.open.as_ref().map(|o| o.id);
        let searching = !self.search.read(cx).value().trim().is_empty();
        let view = cx.entity().downgrade();

        // One calm row: search, new note, and "…" for everything else
        // (tags, trash, import/export). The sidebar already says "Notes".
        let header = if self.show_trash {
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    Button::new("notes-trash-back")
                        .ghost()
                        .small()
                        .icon(Lucide::ArrowLeft)
                        .tooltip("Back to notes")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.set_show_trash(false, window, cx)
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Trash"),
                )
                .when(!self.list.is_empty(), |this| {
                    this.child(
                        Button::new("notes-empty-trash")
                            .ghost()
                            .small()
                            .label("Empty")
                            .tooltip("Delete everything in the trash forever")
                            .on_click(cx.listener(Self::empty_trash)),
                    )
                })
        } else {
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(&self.search).small().cleanable(true).prefix(
                            Icon::new(Lucide::Search)
                                .small()
                                .text_color(theme.muted_foreground),
                        ),
                    ),
                )
                .child(
                    Button::new("notes-new")
                        .ghost()
                        .small()
                        .icon(Lucide::SquarePen)
                        .tooltip_with_action("New note", &crate::actions::NewNote, None)
                        .on_click(cx.listener(|this, _, window, cx| this.new_note(window, cx))),
                )
                .child(self.list_menu_button(cx))
        };

        let in_trash = self.show_trash;
        let rows = self.list.iter().map(|note| {
            let id = note.id;
            let pinned = note.pinned;
            let active = selected == Some(id);
            let view = view.clone();
            let menu_view = view.clone();
            v_flex()
                .id(ElementId::Name(SharedString::from(format!("note-{id}"))))
                .w_full()
                .px_3()
                .py_2()
                .gap_0p5()
                .rounded(theme.radius)
                .border_1()
                .border_color(if active {
                    theme.border
                } else {
                    gpui_kit::transparent_black()
                })
                .when(active, |this| {
                    this.bg(theme.group_box)
                        .when(theme.shadow, |s| s.shadow_xs())
                })
                .when(!active, |this| this.hover(|this| this.bg(theme.list_hover)))
                .on_click(move |_, window, cx| {
                    _ = view.update(cx, |this, cx| this.open_note(id, window, cx));
                })
                .test_support()
                .child(
                    h_flex()
                        .gap_1()
                        .when(note.pinned, |this| {
                            this.child(
                                Icon::new(Lucide::Pin)
                                    .xsmall()
                                    .text_color(theme.muted_foreground),
                            )
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(note.title.clone()),
                        ),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(if note.preview.is_empty() {
                            "No content".to_owned()
                        } else {
                            note.preview.clone()
                        }),
                )
                .context_menu(move |menu, _, _| {
                    note_row_menu(menu, &menu_view, id, pinned, in_trash)
                })
        });

        let empty = match (self.show_trash, searching) {
            _ if self.load_error => empty_state(
                Lucide::TriangleAlert,
                "Couldn't load notes",
                "Your notes are still on disk. Restart Ottero; if this keeps happening, check the data folder in Settings.",
                cx,
            ),
            (true, _) => empty_state(
                Lucide::Trash,
                "Trash is empty",
                "Deleted notes wait here until you remove them.",
                cx,
            ),
            (false, true) => empty_state(
                Lucide::Search,
                "No matches",
                "Try fewer or different words.",
                cx,
            ),
            (false, false) => empty_state(
                Lucide::NotebookPen,
                "No notes yet",
                format!("Create one with {}N.", crate::actions::modifier_label()),
                cx,
            ),
        };

        v_flex()
            .size_full()
            .border_r_1()
            .border_color(theme.border)
            .child(
                v_flex()
                    .px_3()
                    .pt_4()
                    .pb_2()
                    .gap_2()
                    .child(header)
                    .when_some(
                        self.tag_filter.clone().filter(|_| !self.show_trash),
                        |this, tag| {
                            this.child(
                                h_flex().child(
                                    Button::new("clear-tag-filter")
                                        .outline()
                                        .xsmall()
                                        .label(format!("#{tag}"))
                                        .icon(Lucide::X)
                                        .tooltip("Show all notes")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.set_tag_filter(None, window, cx)
                                        })),
                                ),
                            )
                        },
                    ),
            )
            .child(
                div()
                    .id("notes-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .pb_2()
                    .map(|this| {
                        if self.list.is_empty() {
                            this.child(empty)
                        } else {
                            this.child(v_flex().gap_0p5().children(rows))
                        }
                    }),
            )
    }

    /// "…" next to New note: filter by tag, today's note, trash, import/export.
    fn list_menu_button(&self, cx: &Context<Self>) -> impl IntoElement {
        let tags = self.library.read(cx).note_tags();
        let current = self.tag_filter.clone();
        let view = cx.entity().downgrade();
        Button::new("notes-more")
            .ghost()
            .small()
            .icon(Lucide::Ellipsis)
            .selected(current.is_some())
            .tooltip("More")
            .dropdown_menu(move |menu, window, cx| {
                let v = view.clone();
                let run = move |label: &'static str, icon: Lucide, f: fn(&mut NotesView, &mut Window, &mut Context<NotesView>)| {
                    let v = v.clone();
                    PopupMenuItem::new(label).icon(icon).on_click(move |_, window, cx| {
                        _ = v.update(cx, |this, cx| f(this, window, cx));
                    })
                };
                let tags = tags.clone();
                let current = current.clone();
                let tag_view = view.clone();
                menu.item(run("Today's note", Lucide::CalendarDays, |this, w, cx| this.open_daily_note(w, cx)))
                    .separator()
                    .submenu_with_icon(
                        Some(Lucide::Hash.into()),
                        match &current {
                            Some(tag) => format!("Tag: #{tag}"),
                            None => "Filter by tag".to_owned(),
                        },
                        window,
                        cx,
                        move |sub, _, _| {
                            if tags.is_empty() {
                                return sub.label("No tags yet. Type #tag in a note.");
                            }
                            let all = tag_view.clone();
                            let sub = sub
                                .item(PopupMenuItem::new("All notes").checked(current.is_none()).on_click(move |_, window, cx| {
                                    _ = all.update(cx, |this, cx| this.set_tag_filter(None, window, cx));
                                }))
                                .separator();
                            tags.iter().fold(sub, |sub, (tag, count)| {
                                let view = tag_view.clone();
                                let value = tag.clone();
                                sub.item(
                                    PopupMenuItem::new(format!("#{tag}  ·  {count}"))
                                        .checked(current.as_deref() == Some(tag.as_str()))
                                        .on_click(move |_, window, cx| {
                                            _ = view.update(cx, |this, cx| this.set_tag_filter(Some(value.clone()), window, cx));
                                        }),
                                )
                            })
                        },
                    )
                    .separator()
                    .item(run("Import Markdown folder…", Lucide::FolderInput, |this, w, cx| this.import_notes(w, cx)))
                    .item(run("Export as Markdown…", Lucide::FolderOutput, |this, w, cx| this.export_notes(w, cx)))
                    .separator()
                    .item(run("Trash", Lucide::Trash, |this, w, cx| this.set_show_trash(true, w, cx)))
            })
    }

    /// Asks for a folder and imports its `.md` files (Obsidian vaults work).
    pub fn import_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let library = self.library.clone();
        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Import".into()),
        });
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(dir) = paths.into_iter().next() else { return };
            _ = cx.update(|window, cx| {
                match library.update(cx, |lib, cx| lib.import_notes(&dir, cx)) {
                    Ok((imported, skipped)) => {
                        let msg = match skipped {
                            0 => format!("Imported {imported} note(s)."),
                            n => format!("Imported {imported} note(s). Skipped {n} with titles that already exist."),
                        };
                        window.push_notification(msg, cx);
                    }
                    Err(err) => feedback::error_toast(format!("{err:#}"), window, cx),
                }
            });
        })
        .detach();
    }

    /// Asks for a folder and writes every note there as `Title.md`.
    pub fn export_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush(window, cx) {
            return;
        }
        let library = self.library.clone();
        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Export here".into()),
        });
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(dir) = paths.into_iter().next() else {
                return;
            };
            _ = cx.update(|window, cx| match library.read(cx).export_notes(&dir) {
                Ok(n) => window
                    .push_notification(format!("Exported {n} note(s) to {}.", dir.display()), cx),
                Err(err) => feedback::error_toast(format!("{err:#}"), window, cx),
            });
        })
        .detach();
    }

    fn render_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let width = rems(appearance::settings(cx).content_width.max_width_rems());
        let Some(open) = &self.open else {
            return v_flex().flex_1().child(empty_state(
                Lucide::FileText,
                "No note selected",
                "Pick a note from the list, or create a new one.",
                cx,
            ));
        };
        let trashed = open.trashed;
        let pinned = open.pinned;

        let toolbar = h_flex()
            .justify_between()
            .items_center()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(if trashed {
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("In trash · read-only")
                    .into_any_element()
            } else {
                self.render_find_bar(cx)
                    .unwrap_or_else(|| div().into_any_element())
            })
            .child(h_flex().gap_1().map(|this| {
                if trashed {
                    this.child(
                        Button::new("note-restore")
                            .small()
                            .outline()
                            .icon(Lucide::ArchiveRestore)
                            .label("Restore")
                            .on_click(cx.listener(Self::restore_open)),
                    )
                    .child(
                        Button::new("note-delete")
                            .small()
                            .danger()
                            .label("Delete forever")
                            .on_click(cx.listener(Self::delete_open)),
                    )
                } else {
                    // Two controls: format, and "…" for the rest.
                    this.child(self.format_menu(cx))
                        .child(self.note_menu(pinned, cx))
                }
            }));

        let title_text = self.title.read(cx).value();
        // One centered column per child; the editor is as tall as its text, so
        // the page is the only scroll area.
        let column = || div().w_full().max_w(width).mx_auto().px_8();

        let page = v_flex()
            .id("note-scroll")
            .track_scroll(&self.page_scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pt_6()
            .pb_8()
            // Floating menus belong to a spot in the text: close them when it moves away.
            .on_scroll(cx.listener(|this, _, _, cx| {
                this.body.update(cx, |b, cx| b.close_overlays(cx));
            }))
            .map(|this| {
                if trashed {
                    let body_text = self.body.read(cx).text(cx);
                    this.child(column().pb_4().child(
                        div().text_2xl().font_weight(FontWeight::SEMIBOLD).child(
                            if title_text.is_empty() {
                                SharedString::from(ottero_core::note::UNTITLED)
                            } else {
                                title_text.clone()
                            },
                        ),
                    ))
                    .child(column().child(self.render_preview(&body_text, cx)))
                } else {
                    this.child(
                        column().pb_2().child(
                            // Pull the field's inner padding out so the title's
                            // first letter lines up with the body's.
                            div()
                                .ml(-gpui_kit::component::Size::Large.input_px())
                                .child(
                                    Input::new(&self.title)
                                        .appearance(false)
                                        .large()
                                        .text_2xl()
                                        .font_weight(FontWeight::SEMIBOLD),
                                ),
                        ),
                    )
                    .child(column().child(self.body.clone()))
                }
            })
            .when(!self.backlinks.is_empty(), |this| {
                this.child(column().pt_4().child(self.render_backlinks(cx)))
            })
            .when(!self.mentions.is_empty() && !trashed, |this| {
                this.child(column().child(self.render_mentions(cx)))
            })
            .test_support();

        let _ = window;
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(toolbar)
            .child(page)
    }

    /// Scrolls the page so the caret stays visible, with two lines of margin.
    fn follow_caret(&mut self, cx: &mut Context<Self>) {
        let Some(caret) = self.body.read(cx).caret_bounds(cx) else {
            return;
        };
        let viewport = self.page_scroll.bounds();
        if viewport.size.height <= px(0.) {
            return;
        }
        let margin = cx.theme().font_size * 3.;
        let mut offset = self.page_scroll.offset();
        if caret.bottom() + margin > viewport.bottom() {
            offset.y -= caret.bottom() + margin - viewport.bottom();
        } else if caret.top() - margin < viewport.top() {
            offset.y += viewport.top() - (caret.top() - margin);
        } else {
            return;
        }
        let max = self.page_scroll.max_offset().y;
        offset.y = offset.y.min(px(0.)).max(-max);
        self.page_scroll.set_offset(offset);
        cx.notify();
    }

    /// Read-only rendered Markdown for trashed notes, where `[[links]]` and tags are clickable.
    fn render_preview(&self, body: &str, cx: &Context<Self>) -> impl IntoElement {
        let source = preview_markdown(body);
        let view = cx.entity().downgrade();
        TextView::markdown("note-preview", source)
            .selectable(true)
            .on_link_click(move |url, _, window, cx| {
                if let Some(tag) = parse_tag_url(url) {
                    _ = view.update(cx, |this, cx| this.set_tag_filter(Some(tag), window, cx));
                } else if let Some(target) = parse_note_link_url(url) {
                    _ = view.update(cx, |this, cx| this.follow_link(&target, window, cx));
                } else if url.starts_with("https://")
                    || url.starts_with("http://")
                    || url.starts_with("mailto:")
                {
                    cx.open_url(url);
                }
            })
    }

    /// Opens the note a link points to, creating it if needed.
    pub fn follow_link(&mut self, target: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush(window, cx) {
            return;
        }
        let result = self
            .library
            .update(cx, |lib, cx| lib.open_or_create_by_title(target, cx));
        if let Some(id) = feedback::report(result, window, cx) {
            self.open_note(id, window, cx);
        }
    }

    fn render_backlinks(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let view = cx.entity().downgrade();
        v_flex()
            .gap_1()
            .p_4()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.group_box)
            .when(theme.shadow, |s| s.shadow_xs())
            .child(section_label(
                format!(
                    "Linked from {} note{}",
                    self.backlinks.len(),
                    if self.backlinks.len() == 1 { "" } else { "s" }
                ),
                cx,
            ))
            .children(self.backlinks.iter().map(|b| {
                let id = b.id;
                let view = view.clone();
                h_flex()
                    .id(ElementId::Name(SharedString::from(format!(
                        "backlink-{id}"
                    ))))
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(theme.radius)
                    .hover(|this| this.bg(theme.list_hover))
                    .on_click(move |_, window, cx| {
                        _ = view.update(cx, |this, cx| this.open_note(id, window, cx));
                    })
                    .child(
                        Icon::new(Lucide::Link2)
                            .xsmall()
                            .text_color(theme.muted_foreground),
                    )
                    .child(div().text_sm().child(b.title.clone()))
            }))
    }

    fn render_mentions(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let view = cx.entity().downgrade();
        v_flex()
            .gap_1()
            .pt_4()
            .child(section_label(
                format!(
                    "Mentioned in {} note{} without a link",
                    self.mentions.len(),
                    if self.mentions.len() == 1 { "" } else { "s" }
                ),
                cx,
            ))
            .children(self.mentions.iter().map(|m| {
                let id = m.id;
                let open_view = view.clone();
                let link_view = view.clone();
                h_flex()
                    .id(ElementId::Name(SharedString::from(format!("mention-{id}"))))
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(theme.radius)
                    .hover(|this| this.bg(theme.list_hover))
                    .on_click(move |_, window, cx| {
                        _ = open_view.update(cx, |this, cx| this.open_note(id, window, cx));
                    })
                    .child(
                        Icon::new(Lucide::FileText)
                            .xsmall()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .child(m.title.clone()),
                    )
                    .child(
                        Button::new(SharedString::from(format!("link-mention-{id}")))
                            .ghost()
                            .xsmall()
                            .label("Link")
                            .tooltip("Turn the first mention into a [[link]]")
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                _ = link_view
                                    .update(cx, |this, cx| this.link_mention(id, window, cx));
                            }),
                    )
            }))
    }
}

const TAG_URL_PREFIX: &str = "ottero://tag/";

/// Markdown for the preview: `[[links]]` and `#tags` become clickable links.
fn preview_markdown(body: &str) -> String {
    let links = ottero_core::links::wiki_link_spans(body);
    let tags: Vec<_> = ottero_core::tags::tag_spans(body)
        .into_iter()
        .filter(|(r, _)| {
            !links
                .iter()
                .any(|(l, _)| l.start < r.end && r.start < l.end)
        })
        .collect();
    let mut spans: Vec<(std::ops::Range<usize>, String)> = links
        .into_iter()
        .map(|(range, link)| {
            let text = link.alias.clone().unwrap_or_else(|| match &link.heading {
                Some(h) => format!("{} › {h}", link.target),
                None => link.target.clone(),
            });
            (
                range,
                format!(
                    "[{}]({})",
                    escape_link_text(&text),
                    note_link_url(&link.target)
                ),
            )
        })
        .chain(tags.into_iter().map(|(range, tag)| {
            let url = note_link_url(&tag).replacen(
                ottero_core::links::NOTE_LINK_PREFIX,
                TAG_URL_PREFIX,
                1,
            );
            let text = format!("#{}", &body[range.start + 1..range.end]);
            (range, format!("[{}]({url})", escape_link_text(&text)))
        }))
        .collect();
    // ==highlight== → <mark>…</mark> (the renderer understands the HTML tag).
    // Delimiters inside a link or tag are left as typed.
    let marks: Vec<_> = ottero_core::format::highlight_spans(body)
        .into_iter()
        .filter(|(open, close)| {
            !spans.iter().any(|(r, _)| {
                (r.start < open.end && open.start < r.end)
                    || (r.start < close.end && close.start < r.end)
            })
        })
        .flat_map(|(open, close)| [(open, "<mark>".to_owned()), (close, "</mark>".to_owned())])
        .collect();
    spans.extend(marks);
    spans.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(body.len() + 64);
    let mut last = 0;
    for (range, replacement) in spans {
        out.push_str(&body[last..range.start]);
        out.push_str(&replacement);
        last = range.end;
    }
    out.push_str(&body[last..]);
    out
}

fn escape_link_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '[' | ']' | '\\' | '*' | '_' | '`') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn parse_tag_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix(TAG_URL_PREFIX)?;
    parse_note_link_url(&format!("{}{rest}", ottero_core::links::NOTE_LINK_PREFIX))
}

/// Right-click menu for a note in the list. Same order as task menus:
/// open → change → trash.
fn note_row_menu(
    menu: PopupMenu,
    view: &gpui_kit::WeakEntity<NotesView>,
    id: NoteId,
    pinned: bool,
    in_trash: bool,
) -> PopupMenu {
    let run = |label: &'static str,
               icon: Lucide,
               f: fn(&mut NotesView, NoteId, &mut Window, &mut Context<NotesView>)| {
        let view = view.clone();
        PopupMenuItem::new(label)
            .icon(icon)
            .on_click(move |_, window, cx| {
                _ = view.update(cx, |this, cx| f(this, id, window, cx));
            })
    };
    if in_trash {
        return menu
            .item(run("Open", Lucide::FileText, |this, id, window, cx| {
                this.open_note(id, window, cx)
            }))
            .item(run(
                "Restore",
                Lucide::ArchiveRestore,
                |this, id, window, cx| this.restore_note(id, window, cx),
            ));
    }
    menu.item(run("Open", Lucide::FileText, |this, id, window, cx| {
        this.open_note(id, window, cx)
    }))
    .separator()
    .item(if pinned {
        run("Unpin", Lucide::PinOff, |this, id, window, cx| {
            this.set_pinned(id, false, window, cx)
        })
    } else {
        run("Pin to top", Lucide::Pin, |this, id, window, cx| {
            this.set_pinned(id, true, window, cx)
        })
    })
    .separator()
    .item(run(
        "Move to trash",
        Lucide::Trash,
        |this, id, window, cx| this.trash_note(id, window, cx),
    ))
}

impl Focusable for NotesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for NotesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = self.layout.read(cx);
        let panel = layout.panel(NOTES_LIST);
        let toggles = layout.panel_toggles(NOTES_LIST);
        let store = self.layout.clone();
        h_flex()
            .track_focus(&self.focus)
            .size_full()
            .child(
                SidePanel::new(NOTES_LIST.id, PanelSide::Left)
                    .width(panel.width, NOTES_LIST.default)
                    .collapsed(panel.collapsed, toggles)
                    .on_resize(move |width, _, cx| {
                        store.update(cx, |layout, cx| {
                            layout.set_panel_width(NOTES_LIST, width, cx)
                        });
                    })
                    .child(self.render_list(cx)),
            )
            .child(self.render_editor(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::preview_markdown;

    #[test]
    fn preview_turns_highlights_into_mark_tags() {
        assert_eq!(
            preview_markdown("Pay ==rent== by `==code==`"),
            "Pay <mark>rent</mark> by `==code==`"
        );
        let linked = preview_markdown("==See [[Car]]== now");
        assert!(linked.starts_with("<mark>See ["), "{linked}");
        assert!(linked.ends_with("</mark> now"), "{linked}");
    }
}
