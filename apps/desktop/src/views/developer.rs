//! Project context is ordinary local data; credentials remain in the shared
//! vault. The vault item pane/editor is reused, including its lock lifecycle.
use super::{
    Navigate,
    vault_item::{PaneEvent, VaultItemPane},
};
use crate::library::{Library, LibraryEvent};
use crate::vault::{VaultFilter, VaultSort};
use anyhow::Result;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::combobox::{Combobox, ComboboxEvent, ComboboxState};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex, v_virtual_list};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window, div,
    rems,
};
use ottero_core::{
    ProjectId, VaultItemId,
    developer::{DeveloperProject, ProjectLink},
};
use ottero_ui::{
    components::{empty_state, page_header, section_label, segmented},
    feedback,
};

#[derive(Clone)]
struct Choice<V> {
    id: V,
    title: SharedString,
}
impl<V: Clone + PartialEq + 'static> SearchableListItem for Choice<V> {
    type Value = V;
    fn title(&self) -> SharedString {
        self.title.clone()
    }
    fn value(&self) -> &V {
        &self.id
    }
    fn matches(&self, query: &str) -> bool {
        self.title
            .to_lowercase()
            .contains(&query.trim().to_lowercase())
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    Secrets,
}
pub struct DeveloperView {
    library: Entity<Library>,
    pane: Entity<VaultItemPane>,
    workspaces: Vec<DeveloperProject>,
    selected: Option<ProjectId>,
    selected_secret: Option<VaultItemId>,
    tab: Tab,
    archived: bool,
    query: Entity<InputState>,
    name: Entity<InputState>,
    links: Entity<TextareaState>,
    folder: Entity<InputState>,
    existing: Entity<ComboboxState<SearchableVec<Choice<ProjectId>>>>,
    secrets: Entity<ComboboxState<SearchableVec<Choice<VaultItemId>>>>,
    secret_ids: Vec<VaultItemId>,
    creating: bool,
    use_existing: bool,
    editing: bool,
    error: Option<SharedString>,
    load_error: bool,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}
impl EventEmitter<Navigate> for DeveloperView {}
impl DeveloperView {
    pub fn new(library: Entity<Library>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Find a project"));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Project name"));
        let links = cx.new(|cx| {
            TextareaState::new(window, cx).auto_grow(3, 8).placeholder(
                "Repository | https://github.com/you/project\nWebsite | https://example.com",
            )
        });
        let folder = cx.new(|cx| InputState::new(window, cx).placeholder("Optional local folder"));
        let existing = cx.new(|cx| {
            ComboboxState::new(
                SearchableVec::<Choice<ProjectId>>::new(vec![]),
                vec![],
                window,
                cx,
            )
            .searchable(true)
        });
        let secrets = cx.new(|cx| {
            ComboboxState::new(
                SearchableVec::<Choice<VaultItemId>>::new(vec![]),
                vec![],
                window,
                cx,
            )
            .searchable(true)
        });
        let pane = cx.new(|cx| VaultItemPane::new(library.clone(), window, cx));
        let subs = vec![
            cx.subscribe_in(&library, window, |this, _, event, window, cx| {
                if matches!(
                    event,
                    LibraryEvent::TasksChanged
                        | LibraryEvent::NotesChanged
                        | LibraryEvent::DeveloperChanged
                        | LibraryEvent::VaultChanged
                ) {
                    this.reload(window, cx);
                }
            }),
            cx.subscribe(&query, |_, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&name, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) && this.creating {
                    this.create(window, cx);
                }
            }),
            cx.subscribe_in(&secrets, window, |this, _, event, window, cx| {
                if let ComboboxEvent::Change(ids) = event {
                    this.choose_secret(ids.first().copied(), window, cx);
                }
            }),
            cx.subscribe_in(&pane, window, |this, _, event, window, cx| {
                match event {
                    PaneEvent::Created(id) | PaneEvent::Selected(id) => {
                        this.selected_secret = Some(*id)
                    }
                    PaneEvent::Removed => this.selected_secret = None,
                }
                this.reload(window, cx);
            }),
        ];
        let mut this = Self {
            library,
            pane,
            workspaces: vec![],
            selected: None,
            selected_secret: None,
            tab: Tab::Overview,
            archived: false,
            query,
            name,
            links,
            folder,
            existing,
            secrets,
            secret_ids: vec![],
            creating: false,
            use_existing: false,
            editing: false,
            error: None,
            load_error: false,
            focus: cx.focus_handle(),
            _subs: subs,
        };
        this.reload(window, cx);
        this
    }
    pub fn pane(&self) -> &Entity<VaultItemPane> {
        &self.pane
    }
    pub fn selected(&self) -> Option<ProjectId> {
        self.selected
    }
    /// Extends the existing disposable screenshot hook to project environments.
    #[cfg(any(debug_assertions, feature = "perf"))]
    pub fn select_for_dev(&mut self, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .library
            .read(cx)
            .vault()
            .items()
            .iter()
            .find_map(|item| {
                if item.title().eq_ignore_ascii_case(title) {
                    Some((item.fields().environment.as_ref()?.project_id()?, item.id()))
                } else {
                    None
                }
            });
        if let Some((project, id)) = target {
            self.selected = Some(project);
            self.selected_secret = Some(id);
            self.tab = Tab::Secrets;
            self.reload(window, cx);
        }
    }
    pub fn select(&mut self, id: ProjectId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush(window, cx) {
            return;
        }
        self.selected = Some(id);
        self.selected_secret = None;
        self.creating = false;
        self.editing = false;
        self.error = None;
        self.reload(window, cx);
    }
    pub fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.library.read(cx).developer_projects(self.archived) {
            Ok(projects) => {
                self.workspaces = projects;
                self.load_error = false;
            }
            Err(_) => {
                self.workspaces.clear();
                self.load_error = true;
            }
        }
        if !self
            .workspaces
            .iter()
            .any(|w| Some(w.id()) == self.selected)
        {
            self.selected = self.workspaces.first().map(DeveloperProject::id);
            self.selected_secret = None;
        }
        let candidates = match self.library.read(cx).developer_candidates() {
            Ok(candidates) => candidates,
            Err(_) => {
                self.load_error = true;
                Vec::new()
            }
        };
        self.existing.update(cx, |p, cx| {
            p.set_items(
                SearchableVec::new(
                    candidates
                        .into_iter()
                        .map(|p| Choice {
                            id: p.id(),
                            title: p.name().to_owned().into(),
                        })
                        .collect::<Vec<_>>(),
                ),
                window,
                cx,
            )
        });
        self.refresh_secrets(window, cx);
        cx.notify();
    }
    fn refresh_secrets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lib = self.library.read(cx);
        if !lib.vault_is_unlocked() {
            self.secret_ids.clear();
            self.selected_secret = None;
            self.secrets.update(cx, |p, cx| {
                p.set_items(SearchableVec::new(vec![]), window, cx);
                p.set_selected_values(&[], window, cx);
            });
            self.pane.update(cx, |p, cx| p.discard(window, cx));
            return;
        }
        let project = self.selected;
        // Fetch IDs through Library, then read only the encrypted items while unlocked.
        let ids = self.library.update(cx, |l, _| {
            l.vault_list(VaultFilter::All, None, "", VaultSort::Name)
        });
        let items: Vec<_> = ids
            .into_iter()
            .filter_map(|id| {
                let item = self.library.read(cx).vault_item(id)?;
                (item.fields().environment.as_ref()?.project_id() == project && project.is_some())
                    .then(|| Choice {
                        id,
                        title: item.title().to_owned().into(),
                    })
            })
            .collect();
        self.secret_ids = items.iter().map(|i| i.id).collect();
        if self
            .selected_secret
            .is_some_and(|id| !self.secret_ids.contains(&id))
        {
            self.selected_secret = None;
        }
        self.secrets.update(cx, |p, cx| {
            p.set_items(SearchableVec::new(items), window, cx);
            p.set_selected_values(
                &self.selected_secret.into_iter().collect::<Vec<_>>(),
                window,
                cx,
            );
        });
        self.pane
            .update(cx, |p, cx| p.show(self.selected_secret, window, cx));
    }
    fn choose_secret(
        &mut self,
        id: Option<VaultItemId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.flush(window, cx) {
            self.secrets.update(cx, |p, cx| {
                p.set_selected_values(
                    &self.selected_secret.into_iter().collect::<Vec<_>>(),
                    window,
                    cx,
                )
            });
            return;
        }
        self.selected_secret = id;
        self.pane.update(cx, |p, cx| p.show(id, window, cx));
        cx.notify();
    }
    pub fn begin_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush(window, cx) {
            return;
        }
        self.creating = true;
        self.editing = false;
        self.use_existing = false;
        self.error = None;
        self.name.update(cx, |i, cx| {
            i.set_value("", window, cx);
            i.focus(window, cx);
        });
        cx.notify();
    }
    pub fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let existing = if self.use_existing {
            match self.existing.read(cx).selected_value() {
                Some(id) => Some(id),
                None => {
                    self.error = Some("Choose an existing task project.".into());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let name = self.name.read(cx).value();
        let result = self
            .library
            .update(cx, |l, cx| l.create_developer_project(&name, existing, cx));
        match result {
            Ok(id) => {
                self.creating = false;
                self.archived = false;
                self.selected = Some(id);
                self.reload(window, cx);
            }
            Err(err) => self.error = Some(err.to_string().into()),
        }
        cx.notify();
    }
    fn begin_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(w) = self.current().cloned() else {
            return;
        };
        self.editing = true;
        self.error = None;
        self.name.update(cx, |i, cx| {
            i.set_value(w.project().name().to_owned(), window, cx)
        });
        self.links.update(cx, |i, cx| {
            i.set_value(
                w.links()
                    .iter()
                    .map(|l| format!("{} | {}", l.label(), l.url()))
                    .collect::<Vec<_>>()
                    .join("\n"),
                window,
                cx,
            )
        });
        self.folder
            .update(cx, |i, cx| i.set_value(w.folder().to_owned(), window, cx));
        cx.notify();
    }
    fn save_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.selected else {
            return false;
        };
        let parsed: Result<Vec<_>> = self
            .links
            .read(cx)
            .value()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let (label, url) = line
                    .split_once('|')
                    .ok_or_else(|| anyhow::anyhow!("Use Label | https://… for each link."))?;
                Ok(ProjectLink::new(label.trim(), url.trim())?)
            })
            .collect();
        let result = parsed.and_then(|links| {
            self.library.update(cx, |l, cx| {
                l.update_developer_project(
                    id,
                    &self.name.read(cx).value(),
                    links,
                    &self.folder.read(cx).value(),
                    cx,
                )
            })
        });
        match result {
            Ok(()) => {
                self.editing = false;
                self.error = None;
                self.reload(window, cx);
                true
            }
            Err(err) => {
                self.error = Some(err.to_string().into());
                cx.notify();
                false
            }
        }
    }
    pub fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.editing && !self.save_profile(window, cx) {
            return false;
        }
        if self.pane.read(cx).is_editing() {
            self.pane.update(cx, |p, cx| p.save(window, cx));
            if self.pane.read(cx).is_editing() {
                return false;
            }
        }
        true
    }
    fn current(&self) -> Option<&DeveloperProject> {
        self.workspaces
            .iter()
            .find(|p| Some(p.id()) == self.selected)
    }
    pub fn new_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush(window, cx) {
            return;
        }
        if !self.library.read(cx).vault_is_unlocked() {
            // The Secrets tab owns the lock guidance, which disappears on unlock.
            self.creating = false;
            self.error = None;
            self.tab = Tab::Secrets;
            cx.notify();
            return;
        }
        let Some(w) = self.current().filter(|w| !w.is_archived()).cloned() else {
            self.error = Some("Choose an active project first.".into());
            cx.notify();
            return;
        };
        self.tab = Tab::Secrets;
        self.creating = false;
        self.error = None;
        self.selected_secret = None;
        self.pane.update(cx, |p, cx| {
            p.start_environment(w.id(), w.project().name(), window, cx)
        });
        cx.notify();
    }
    fn archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush(window, cx) {
            return;
        }
        let Some(w) = self.current().cloned() else {
            return;
        };
        let id = w.id();
        let archived = !w.is_archived();
        if self
            .library
            .update(cx, |l, cx| l.archive_developer_project(id, archived, cx))
            .is_err()
        {
            self.error = Some("Couldn't change project status.".into());
            cx.notify();
            return;
        }
        let library = self.library.clone();
        feedback::undo_toast(
            if archived {
                "Project archived. Notes, tasks and secrets kept."
            } else {
                "Project restored."
            },
            window,
            cx,
            move |window, cx| {
                let result =
                    library.update(cx, |l, cx| l.archive_developer_project(id, !archived, cx));
                super::task_row::notify_err(result, window, cx);
            },
        );
        self.reload(window, cx);
    }
    fn setup_note(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            return;
        };
        match self
            .library
            .update(cx, |l, cx| l.developer_setup_note(id, cx))
        {
            Ok(id) => cx.emit(Navigate::OpenNote(id)),
            Err(err) => {
                self.error = Some(err.to_string().into());
                cx.notify();
            }
        }
    }
    fn form(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        v_flex().gap_3().max_w(rems(36.)).p_6()
            .on_key_down(cx.listener(|s,e:&gpui_kit::KeyDownEvent,w,cx| {
                if e.keystroke.key == "escape" {cx.stop_propagation();s.cancel_form(w,cx);}
            }))
            .child(div().text_lg().child(if self.creating{"New developer project"}else{"Edit project"}))
            .when(self.creating,|s|s.child(segmented("developer-create-mode",&[(false,"New project"),(true,"Existing task project")],self.use_existing,{
                let view=cx.entity();move |existing,_,cx|view.update(cx,|s,cx|{s.use_existing=existing;cx.notify();})
            })))
            .child(if self.creating&&self.use_existing{Combobox::new(&self.existing).placeholder("Choose a task project").search_placeholder("Find a project").menu_max_h(rems(16.)).into_any_element()}
                else{Input::new(&self.name).id("developer-name").into_any_element()})
            .when(!self.creating,|s|s.child(section_label("Useful links",cx)).child(div().id("developer-links").test_support().child(Textarea::new(&self.links)))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("One Label | https://… per line. Store credentials in Secrets, not links."))
                .child(section_label("Local folder",cx)).child(Input::new(&self.folder).id("developer-folder")))
            .when_some(self.error.clone(),|s,e|s.child(div().text_sm().text_color(cx.theme().danger).child(e)))
            .child(h_flex().gap_2()
                .child(Button::new("developer-save").outline().label(if self.creating{"Create project"}else{"Save"}).on_click(cx.listener(|s,_,w,cx|{if s.creating{s.create(w,cx);}else{s.save_profile(w,cx);}})))
                .child(Button::new("developer-cancel").ghost().label("Cancel").on_click(cx.listener(|s,_,w,cx|s.cancel_form(w,cx)))))
            .into_any_element()
    }
    fn cancel_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.creating = false;
        self.editing = false;
        self.error = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }
    pub fn toggle_archived(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush(window, cx) {
            return;
        }
        self.archived = !self.archived;
        self.creating = false;
        self.error = None;
        self.selected = None;
        self.reload(window, cx);
    }
    fn overview(
        &self,
        workspace: &DeveloperProject,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let id = workspace.id();
        let theme = cx.theme().clone();
        let card = || {
            v_flex()
                .w_full()
                .p_4()
                .gap_3()
                .rounded(theme.radius_lg)
                .border_1()
                .border_color(theme.border)
                .bg(theme.group_box)
                .when(theme.shadow, |s| s.shadow_xs())
        };
        v_flex()
            .p_6()
            .gap_4()
            .max_w(rems(46.))
            .child(
                card().child(section_label("Work", cx)).child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new("developer-setup-note")
                                .outline()
                                .small()
                                .icon(Lucide::NotebookPen)
                                .label("Setup note")
                                .on_click(cx.listener(|s, _, _, cx| s.setup_note(cx))),
                        )
                        .child(
                            Button::new("developer-tasks")
                                .ghost()
                                .small()
                                .icon(Lucide::ListTodo)
                                .label("Project tasks")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(Navigate::ProjectTasks(id))
                                })),
                        ),
                ),
            )
            .child(
                card()
                    .gap_2()
                    .child(section_label("Resources", cx))
                    .children(workspace.links().iter().enumerate().map(|(index, link)| {
                        let url = link.url().to_owned();
                        Button::new(SharedString::from(format!("developer-link-{id}-{index}")))
                            .ghost()
                            .w_full()
                            .h(rems(3.25))
                            .accessibility_label(link.label().to_owned())
                            .tooltip(url.clone())
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .gap_3()
                                    .items_center()
                                    .child(
                                        Icon::new(Lucide::ExternalLink)
                                            .small()
                                            .text_color(cx.theme().muted_foreground),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .truncate()
                                                    .child(link.label().to_owned()),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .truncate()
                                                    .child(url.clone()),
                                            ),
                                    ),
                            )
                            .on_click(move |_, _, cx| cx.open_url(&url))
                    }))
                    .when(workspace.links().is_empty(), |s| {
                        s.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    "Add a repository, website or dashboard from the project menu.",
                                ),
                        )
                    }),
            )
            .when(!workspace.folder().is_empty(), |s| {
                let folder = std::path::PathBuf::from(workspace.folder());
                s.child(
                    card()
                        .gap_2()
                        .child(section_label("Local folder", cx))
                        .child(
                            Button::new("developer-open-folder")
                                .ghost()
                                .w_full()
                                .accessibility_label("Open project folder")
                                .child(
                                    h_flex()
                                        .w_full()
                                        .min_w_0()
                                        .gap_3()
                                        .items_center()
                                        .child(
                                            Icon::new(Lucide::FolderKanban)
                                                .small()
                                                .text_color(cx.theme().muted_foreground),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .text_sm()
                                                .truncate()
                                                .child(workspace.folder().to_owned()),
                                        ),
                                )
                                .tooltip(workspace.folder().to_owned())
                                .on_click(move |_, _, cx| cx.reveal_path(&folder)),
                        ),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Project details are local app data. Keep credentials in Secrets."),
            )
            .into_any_element()
    }
}
impl Focusable for DeveloperView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for DeveloperView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.query.read(cx).value().to_lowercase();
        let indices: Vec<_> = self
            .workspaces
            .iter()
            .enumerate()
            .filter_map(|(i, w)| {
                w.project()
                    .name()
                    .to_lowercase()
                    .contains(&query)
                    .then_some(i)
            })
            .collect();
        let sizes = std::rc::Rc::new(
            indices
                .iter()
                .map(|_| gpui_kit::size(gpui_kit::px(0.), window.rem_size() * 2.5))
                .collect(),
        );
        let list = v_virtual_list(
            cx.entity(),
            "developer-projects",
            sizes,
            move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .filter_map(|ix| indices.get(ix).and_then(|ix| this.workspaces.get(*ix)))
                    .map(|p| {
                        let id = p.id();
                        Button::new(SharedString::from(format!("developer-project-{id}")))
                            .ghost()
                            .w_full()
                            .h(rems(2.25))
                            .accessibility_label(p.project().name().to_owned())
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        Icon::new(Lucide::FolderKanban)
                                            .small()
                                            .text_color(cx.theme().muted_foreground),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .truncate()
                                            .child(p.project().name().to_owned()),
                                    ),
                            )
                            .when(this.selected == Some(id), |s| {
                                s.bg(cx.theme().sidebar_accent)
                                    .font_weight(FontWeight::MEDIUM)
                            })
                            .on_click(cx.listener(move |s, _, w, cx| s.select(id, w, cx)))
                            .into_any_element()
                    })
                    .collect()
            },
        )
        .flex_1()
        .min_h_0();
        let archived = self.archived;
        let left = v_flex()
            .w(rems(15.))
            .min_w(rems(12.))
            .flex_shrink_0()
            .h_full()
            .border_r_1()
            .border_color(cx.theme().border)
            .p_3()
            .gap_3()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Projects"),
                    )
                    .child(
                        Button::new("developer-new")
                            .ghost()
                            .small()
                            .icon(Lucide::Plus)
                            .tooltip("New project…")
                            .on_click(|_, w, cx| {
                                w.dispatch_action(Box::new(crate::actions::NewDeveloperProject), cx)
                            }),
                    )
                    .child(
                        Button::new("developer-more")
                            .ghost()
                            .small()
                            .icon(Lucide::Ellipsis)
                            .tooltip("Project list")
                            .dropdown_menu(move |menu, _, _| {
                                menu.menu_with_check(
                                    "Show archived projects",
                                    archived,
                                    Box::new(crate::actions::ToggleArchivedDeveloperProjects),
                                )
                            }),
                    ),
            )
            .child(Input::new(&self.query).id("developer-search").small())
            .when(self.archived, |s| {
                s.child(section_label("Archived projects", cx))
            })
            .child(if self.load_error {
                empty_state(
                    Lucide::TriangleAlert,
                    "Couldn't load projects",
                    "Reopen Developer to retry.",
                    cx,
                )
                .into_any_element()
            } else if self.workspaces.is_empty() {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.archived {
                        "No archived projects"
                    } else {
                        "No projects yet. Add a new project or use an existing task project."
                    })
                    .into_any_element()
            } else {
                list.into_any_element()
            });
        let content = if self.creating || self.editing {
            self.form(cx)
        } else if let Some(workspace) = self.current().cloned() {
            let body = match self.tab {
                Tab::Overview => self.overview(&workspace, cx),
                Tab::Secrets => {
                    if !self.library.read(cx).vault_is_unlocked() {
                        v_flex().p_8().gap_3().child(section_label("Project secrets",cx))
                            .child(div().text_sm().child("Unlock Passwords to view or add this project's secrets. Your existing master password is used."))
                            .child(Button::new("developer-unlock").outline().label("Open Passwords").on_click(|_,w,cx|w.dispatch_action(Box::new(crate::actions::GoPasswords),cx))).into_any_element()
                    } else {
                        v_flex().size_full().gap_2()
                            .child(h_flex().px_6().py_3().gap_2()
                                .child(div().flex_1().child(Combobox::new(&self.secrets).placeholder("Choose an environment").search_placeholder("Find environment").menu_max_h(rems(16.))))
                                .when(!workspace.is_archived(),|s|s.child(Button::new("developer-new-environment").outline().small().label("New environment…").on_click(|_,w,cx|w.dispatch_action(Box::new(crate::actions::NewProjectSecrets),cx)))))
                            .child(div().flex_1().min_h_0().child(if self.selected_secret.is_some() || self.pane.read(cx).is_editing() {self.pane.clone().into_any_element()}
                                else {v_flex().p_6().gap_2().child(section_label("Environment sets",cx)).child(div().text_sm().text_color(cx.theme().muted_foreground).child("Choose an environment or create one for development, staging or production. Values stay in your password vault.")).into_any_element()})).into_any_element()
                    }
                }
            };
            v_flex()
                .size_full()
                .min_w_0()
                .child(
                    h_flex()
                        .w_full()
                        .flex_wrap()
                        .px_6()
                        .py_4()
                        .gap_2()
                        .items_center()
                        .justify_between()
                        .child(page_header(
                            workspace.project().name().to_owned(),
                            Some(if workspace.is_archived() {
                                "Archived".into()
                            } else {
                                "Project workspace".into()
                            }),
                            cx,
                        ))
                        .child(
                            Button::new("developer-project-menu")
                                .ghost()
                                .small()
                                .icon(Lucide::Ellipsis)
                                .tooltip("Project actions")
                                .dropdown_menu({
                                    let view = cx.entity().downgrade();
                                    move |menu, _, _| {
                                        let (edit, archive) = (view.clone(), view.clone());
                                        menu.item(
                                            PopupMenuItem::new("Edit project…")
                                                .disabled(workspace.is_archived())
                                                .on_click(move |_, w, cx| {
                                                    _ = edit
                                                        .update(cx, |s, cx| s.begin_edit(w, cx));
                                                }),
                                        )
                                        .separator()
                                        .item(
                                            PopupMenuItem::new(if workspace.is_archived() {
                                                "Restore project"
                                            } else {
                                                "Archive project"
                                            })
                                            .on_click(move |_, w, cx| {
                                                _ = archive.update(cx, |s, cx| s.archive(w, cx));
                                            }),
                                        )
                                    }
                                }),
                        ),
                )
                .child(h_flex().px_6().pb_3().child(segmented(
                    "developer-tabs",
                    &[(Tab::Overview, "Overview"), (Tab::Secrets, "Secrets")],
                    self.tab,
                    {
                        let view = cx.entity();
                        move |tab, w, cx| {
                            view.update(cx, |s, cx| {
                                if s.flush(w, cx) {
                                    s.tab = tab;
                                    cx.notify();
                                }
                            })
                        }
                    },
                )))
                .when_some(self.error.clone(), |s, e| {
                    s.child(
                        div()
                            .px_6()
                            .text_sm()
                            .text_color(cx.theme().danger)
                            .child(e),
                    )
                })
                .child(
                    div()
                        .id("developer-detail")
                        .flex_1()
                        .min_h_0()
                        .when(self.tab == Tab::Overview, |s| s.overflow_y_scroll())
                        .child(body),
                )
                .into_any_element()
        } else {
            empty_state(
                Lucide::Code,
                "Your development projects",
                "Keep setup notes, useful links and encrypted environments together.",
                cx,
            )
            .into_any_element()
        };
        h_flex()
            .id("developer-screen")
            .track_focus(&self.focus)
            .size_full()
            .child(left)
            .child(div().flex_1().min_w_0().h_full().child(content))
    }
}
