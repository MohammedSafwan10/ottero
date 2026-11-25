//! Passwords screen: setup → locked → unlocked.
//!
//! * **Setup** (no vault yet): choose a master password with a live strength
//!   meter, confirm it, acknowledge it can't be recovered.
//! * **Locked**: one password field. Argon2 runs in the background; the
//!   button shows progress and the field keeps its text on a wrong password.
//! * **Ottero Cloud** (ADR 0014, [`Screen`]): signed in, the vault opens
//!   with the account. A master-password vault asks for that password once
//!   to link; a new vault is set up without one; a linked vault's locked
//!   screen is "Unlock Ottero" (the account password opens both), or says
//!   plainly when another account or no account is signed in.
//! * **Unlocked**: master–detail. Left: search, New, a virtualized list
//!   (fine with thousands of items). Right: the item (`vault_item.rs`).
//!
//! The list filter comes from the sidebar (All, Favorites, kinds, Password
//! health, Trash). Auto-lock: a 10-second tick locks after the idle time in
//! Settings → Passwords; any key or pointer activity in the app resets it
//! (`Library::touch_vault`, from the shell).
//!
//! Keyboard (list focused): ↑/↓ select, ⌘C copy password, ⇧⌘C copy
//! username, ⌥⌘C copy the one-time code, Enter edit, ⌘⌫ move to trash.

use std::ops::Range;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, ScrollStrategy,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task as GpuiTask,
    TestSupportExt as _, Window, div, rems,
};
use ottero_core::VaultItemId;
use ottero_core::vault::health::{Strength, estimate};
use ottero_core::vault::lock::{LockState, Opener};
use ottero_core::vault::{ItemKind, VaultItem};
use ottero_crypto::SecretString;
use ottero_ui::components::empty_state;

use crate::actions::{
    VAULT_CONTEXT, VaultCopyCode, VaultCopyPassword, VaultCopyUsername, VaultEditSelected,
    VaultNextItem, VaultPreviousItem, VaultTrashSelected,
};
use crate::library::{Library, LibraryEvent};
use crate::vault::{AccountUnlock, LinkResult, VaultFilter, VaultSort};
use crate::vault_prefs;
use crate::views::vault_item::{PaneEvent, VaultItemPane, avatar, copy_field};
use gpui_kit::component::{VirtualListScrollHandle, v_virtual_list};
use ottero_core::vault::template::Template;
use ottero_core::vault::totp;

/// Shortest master password we accept (zxcvbn must also say "Fair" or better).
pub const MIN_MASTER_CHARS: usize = 10;
const ROW_HEIGHT: f32 = 3.25;
const AUTO_LOCK_TICK: Duration = Duration::from_secs(10);
/// A tick arriving this late means the Mac slept (timers don't run then):
/// lock on wake. Ottero can't observe the lock screen itself (GPUI has no
/// hook and the workspace forbids `unsafe` FFI), so this plus "Lock when in
/// background" is the defense.
const SLEEP_GAP: Duration = Duration::from_secs(60);

/// What the Passwords screen shows, from the vault and the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// No vault, not signed in: choose a master password.
    Setup,
    /// No vault, signed in: set up under the account (no master password).
    SetupWithAccount,
    /// A master-password vault, not signed in (or the account is locked).
    Locked,
    /// A master-password vault while the account is open: link it.
    Link,
    /// A linked vault and its account is signed in but locked.
    UnlockOttero,
    /// A linked vault and nobody is signed in (its email).
    SignedOut(String),
    /// A linked vault and another account is signed in (linked, signed in).
    OtherAccount(String, String),
    /// Same email, but the open account key doesn't open it.
    WrongKey,
    Recovery,
    Unlocked,
}

/// One line in the item list.
#[derive(Debug, Clone, PartialEq)]
enum Row {
    Header(SharedString),
    Item(VaultItemId),
}

/// Show "Recent" above the list when the vault has more items than this.
const RECENT_FROM: usize = 12;
const RECENT_MAX: usize = 4;
/// Show A–Z headers when a name-sorted list is longer than this.
const LETTERS_FROM: usize = 30;
const HEADER_HEIGHT: f32 = 1.75;

pub struct PasswordsView {
    library: Entity<Library>,
    filter: VaultFilter,
    /// Tag chosen in the sidebar (narrows any filter).
    tag: Option<String>,
    search: Entity<InputState>,
    /// The list in navigation order (↑/↓).
    ids: Vec<VaultItemId>,
    /// What the list draws: items plus section headers.
    rows: Vec<Row>,
    selected: Option<VaultItemId>,
    scroll: VirtualListScrollHandle,
    /// Shown once after setup or when a new key is made; zeroed on drop.
    recovery: Option<ottero_crypto::Zeroizing<String>>,
    recovery_saved: bool,
    /// Re-renders codes each second in the one-time codes view.
    _codes_tick: Option<GpuiTask<()>>,
    pane: Entity<VaultItemPane>,
    // Setup
    master: Entity<InputState>,
    confirm: Entity<InputState>,
    acknowledged: bool,
    // Unlock
    unlock: Entity<InputState>,
    /// The Ottero account password ("Unlock Ottero", setup while locked).
    account_password: Entity<InputState>,
    error: Option<SharedString>,
    busy: bool,
    list_focus: FocusHandle,
    focus: FocusHandle,
    _tick: GpuiTask<()>,
    /// Wall-clock time of the last auto-lock tick (sleep detection).
    last_tick: std::time::SystemTime,
    _subscriptions: Vec<Subscription>,
}

impl PasswordsView {
    pub fn new(library: Entity<Library>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let masked = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder(placeholder)
            })
        };
        let master = masked("Master password", window, cx);
        let confirm = masked("Type it again", window, cx);
        let unlock = masked("Master password", window, cx);
        let account_password = masked("Ottero password", window, cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search vault"));
        let pane = cx.new(|cx| VaultItemPane::new(library.clone(), window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&library, window, |this, _, event, window, cx| {
                if *event == LibraryEvent::VaultChanged {
                    this.refresh(window, cx);
                }
            }),
            cx.subscribe_in(&search, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refresh(window, cx);
                }
            }),
            cx.subscribe_in(&master, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.error = None;
                    cx.notify();
                }
            }),
            cx.subscribe_in(&confirm, window, |this, _, event, window, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => this.create_vault(window, cx),
                _ => {}
            }),
            cx.subscribe_in(&unlock, window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => this.unlock(window, cx),
                InputEvent::Change => {
                    this.error = None;
                    cx.notify();
                }
                _ => {}
            }),
            cx.subscribe_in(
                &account_password,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.submit_account_password(window, cx),
                    InputEvent::Change => {
                        this.error = None;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(&library, window, |this, _, event, _, cx| {
                if *event == LibraryEvent::AccountChanged {
                    // The screen depends on the account (link, unlock, other account).
                    this.error = None;
                    cx.notify();
                }
            }),
            cx.subscribe_in(&pane, window, |this, _, event, window, cx| match event {
                PaneEvent::Created(id) | PaneEvent::Selected(id) => {
                    this.selected = Some(*id);
                    this.refresh(window, cx);
                    this.scroll_to_selection();
                }
                PaneEvent::Removed => {
                    this.selected = None;
                    this.refresh(window, cx);
                }
            }),
        ];
        let tick = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(AUTO_LOCK_TICK).await;
                let alive = this.update(cx, |this, cx| this.check_auto_lock(cx));
                if alive.is_err() {
                    break;
                }
            }
        });
        let mut this = Self {
            library,
            filter: VaultFilter::All,
            search,
            ids: Vec::new(),
            rows: Vec::new(),
            tag: None,
            selected: None,
            scroll: VirtualListScrollHandle::new(),
            recovery: None,
            recovery_saved: false,
            _codes_tick: None,
            pane,
            master,
            confirm,
            acknowledged: false,
            unlock,
            account_password,
            error: None,
            busy: false,
            list_focus: cx.focus_handle(),
            focus: cx.focus_handle(),
            _tick: tick,
            last_tick: std::time::SystemTime::now(),
            _subscriptions: subscriptions,
        };
        this.refresh(window, cx);
        this
    }

    pub fn filter(&self) -> VaultFilter {
        self.filter
    }

    pub fn ids(&self) -> &[VaultItemId] {
        &self.ids
    }

    pub fn selected(&self) -> Option<VaultItemId> {
        self.selected
    }

    pub fn pane(&self) -> &Entity<VaultItemPane> {
        &self.pane
    }

    /// The message under the current form (wrong password and the like).
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn master_input(&self) -> &Entity<InputState> {
        &self.master
    }

    #[doc(hidden)]
    pub fn select_for_dev(&mut self, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.ids.iter().copied().find(|id| {
            self.library
                .read(cx)
                .vault_item(*id)
                .is_some_and(|i| i.title().eq_ignore_ascii_case(title))
        });
        self.select(id, window, cx);
    }

    pub fn set_filter(&mut self, filter: VaultFilter, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter != filter || self.tag.is_some() {
            if !self.flush_edit(window, cx) {
                return;
            }
            self.filter = filter;
            self.tag = None;
            self.selected = None;
            self.update_codes_tick(window, cx);
            self.refresh(window, cx);
        }
    }

    pub fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    /// Shows items with `tag` (from the sidebar), across all kinds.
    pub fn set_tag(&mut self, tag: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush_edit(window, cx) {
            return;
        }
        self.filter = VaultFilter::All;
        self.tag = tag;
        self.selected = None;
        self.update_codes_tick(window, cx);
        self.refresh(window, cx);
    }

    /// The recovery key waiting to be saved (setup or "new recovery key").
    pub fn recovery_key(&self) -> Option<&str> {
        self.recovery.as_deref().map(String::as_str)
    }

    pub fn show_recovery_key(
        &mut self,
        key: ottero_crypto::Zeroizing<String>,
        cx: &mut Context<Self>,
    ) {
        self.recovery = Some(key);
        self.recovery_saved = false;
        cx.notify();
    }

    fn update_codes_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter == VaultFilter::Codes {
            if self._codes_tick.is_none() {
                self._codes_tick = Some(cx.spawn_in(window, async move |this, cx| {
                    loop {
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                }));
            }
        } else {
            self._codes_tick = None;
        }
    }

    /// What the screen shows now.
    pub fn screen(&self, cx: &App) -> Screen {
        let lib = self.library.read(cx);
        let account = lib.account();
        let signed_in = account.signed_in_email();
        let open = account.is_unlocked() && signed_in.is_some();
        match lib.vault_state() {
            LockState::Uninitialized if signed_in.is_some() => Screen::SetupWithAccount,
            LockState::Uninitialized => Screen::Setup,
            LockState::Unlocked if self.recovery.is_some() => Screen::Recovery,
            LockState::Unlocked => Screen::Unlocked,
            LockState::Locked | LockState::Unlocking => match lib.vault_opener() {
                Some(Opener::Account) => {
                    let linked = lib.vault_linked_email().unwrap_or_default().to_owned();
                    match signed_in {
                        None => Screen::SignedOut(linked),
                        Some(email) if email != linked => Screen::OtherAccount(linked, email),
                        Some(_) if !account.is_unlocked() => Screen::UnlockOttero,
                        Some(_) => match lib.vault_account_problem() {
                            Some(AccountUnlock::WrongKey) => Screen::WrongKey,
                            _ => Screen::UnlockOttero,
                        },
                    }
                }
                _ if open => Screen::Link,
                _ => Screen::Locked,
            },
        }
    }

    /// Where focus goes when the screen opens: the field that matters.
    pub fn focus_primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = match self.screen(cx) {
            Screen::Setup => &self.master,
            Screen::Locked | Screen::Link => &self.unlock,
            Screen::UnlockOttero | Screen::SetupWithAccount => &self.account_password,
            Screen::Unlocked => &self.search,
            Screen::SignedOut(_)
            | Screen::OtherAccount(..)
            | Screen::WrongKey
            | Screen::Recovery => {
                window.focus(&self.focus, cx);
                return;
            }
        };
        input.update(cx, |i, cx| i.focus(window, cx));
    }

    fn check_auto_lock(&mut self, cx: &mut Context<Self>) {
        let now = std::time::SystemTime::now();
        let gap = now.duration_since(self.last_tick).unwrap_or(Duration::MAX);
        self.last_tick = now;
        let lib = self.library.read(cx);
        if lib.vault_state() != LockState::Unlocked && !lib.account_key_open() {
            return;
        }
        let idle = vault_prefs::get(cx).auto_lock.expired(lib.vault_idle());
        let slept = gap >= SLEEP_GAP;
        if idle || slept {
            self.library.update(cx, |lib, cx| lib.lock_vault(cx));
        }
    }

    #[doc(hidden)]
    pub fn pretend_slept_for_tests(&mut self) {
        self.last_tick = std::time::SystemTime::UNIX_EPOCH;
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.library.read(cx).vault_state();
        if state != LockState::Unlocked {
            // Locked: drop every trace of plaintext the screen holds, drafts too.
            self.ids.clear();
            self.rows.clear();
            self.selected = None;
            self.tag = None;
            self._codes_tick = None;
            self.recovery = None;
            self.recovery_saved = false;
            self.pane.update(cx, |p, cx| p.discard(window, cx));
            self.search.update(cx, |i, cx| i.set_value("", window, cx));
            cx.notify();
            return;
        }
        let query = self.search.read(cx).value().to_string();
        let filter = self.filter;
        let tag = self.tag.clone();
        let sort = vault_prefs::get(cx).sort;
        self.ids = self.library.update(cx, |lib, _| {
            lib.vault_list(filter, tag.as_deref(), &query, sort)
        });
        self.rows = self.build_rows(query.trim().is_empty(), sort, cx);
        if self.selected.is_some_and(|id| !self.ids.contains(&id)) {
            // Keep a just-edited item open even if it no longer matches.
            let exists = self
                .selected
                .is_some_and(|id| self.library.read(cx).vault_item(id).is_some());
            if !exists {
                self.selected = None;
            }
        }
        let selected = self.selected;
        self.pane.update(cx, |p, cx| p.show(selected, window, cx));
        cx.notify();
    }

    /// Items plus headers: "Recent" on a big, unfiltered vault; A–Z letters
    /// on long name-sorted lists. Search results are one ranked list.
    fn build_rows(&self, no_query: bool, sort: VaultSort, cx: &App) -> Vec<Row> {
        let lib = self.library.read(cx);
        let mut rows = Vec::with_capacity(self.ids.len() + 8);
        let browsing = no_query && self.tag.is_none();
        if browsing && self.filter == VaultFilter::All && self.ids.len() > RECENT_FROM {
            let recent = lib.vault_recent(RECENT_MAX);
            if !recent.is_empty() {
                rows.push(Row::Header("Recently used".into()));
                rows.extend(recent.into_iter().map(Row::Item));
                rows.push(Row::Header(
                    if sort == VaultSort::Name {
                        "All items"
                    } else {
                        sort.label()
                    }
                    .into(),
                ));
            }
        }
        let letters = no_query && sort == VaultSort::Name && self.ids.len() > LETTERS_FROM;
        let mut last: Option<char> = None;
        for id in &self.ids {
            if letters {
                let initial = lib
                    .vault_item(*id)
                    .and_then(|i| i.title().chars().next())
                    .map(|c| {
                        if c.is_alphabetic() {
                            c.to_uppercase().next().unwrap_or(c)
                        } else {
                            '#'
                        }
                    });
                if initial.is_some() && initial != last {
                    last = initial;
                    rows.push(Row::Header(initial.unwrap_or('#').to_string().into()));
                }
            }
            rows.push(Row::Item(*id));
        }
        rows
    }

    fn select(&mut self, id: Option<VaultItemId>, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected != id && !self.flush_edit(window, cx) {
            return;
        }
        self.selected = id;
        self.pane.update(cx, |p, cx| p.show(id, window, cx));
        self.scroll_to_selection();
        cx.notify();
    }

    // Navigation must keep an invalid draft open so the user can correct it.
    pub(crate) fn flush_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.pane.read(cx).is_editing() {
            self.pane.update(cx, |p, cx| p.save(window, cx));
        }
        !self.pane.read(cx).is_editing()
    }

    fn scroll_to_selection(&self) {
        // The last row for the item: the main list, not the "Recent" copy.
        if let Some(ix) = self
            .selected
            .and_then(|id| self.rows.iter().rposition(|r| *r == Row::Item(id)))
        {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
    }

    fn step(&mut self, dir: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.ids.is_empty() {
            return;
        }
        let current = self
            .selected
            .and_then(|id| self.ids.iter().position(|i| *i == id));
        let next = match current {
            None if dir > 0 => 0,
            None => self.ids.len() - 1,
            Some(i) => (i as i64 + dir).clamp(0, self.ids.len() as i64 - 1) as usize,
        };
        let id = self.ids[next];
        self.select(Some(id), window, cx);
    }

    // ---- Setup and unlock -------------------------------------------------

    fn master_estimate(&self, cx: &App) -> Option<ottero_core::vault::health::Estimate> {
        let text = self.master.read(cx).value();
        (!text.is_empty()).then(|| estimate(&text, &["ottero", "vault", "password"]))
    }

    /// Why "Create vault" is disabled, or `None` when it can run.
    fn setup_problem(&self, cx: &App) -> Option<&'static str> {
        let master = self.master.read(cx).value();
        let confirm = self.confirm.read(cx).value();
        if master.chars().count() < MIN_MASTER_CHARS {
            return Some("Use at least 10 characters.");
        }
        if self
            .master_estimate(cx)
            .is_some_and(|e| e.strength < Strength::Fair)
        {
            return Some("Choose something harder to guess.");
        }
        if confirm.is_empty() {
            return Some("Type the password again to confirm it.");
        }
        if master != confirm {
            return Some("The two passwords don't match.");
        }
        if !self.acknowledged {
            return Some("Confirm you understand it can't be recovered.");
        }
        None
    }

    fn create_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.setup_problem(cx).is_some() {
            return;
        }
        let password = SecretString::from(self.master.read(cx).value().to_string());
        self.busy = true;
        cx.notify();
        let task = self
            .library
            .update(cx, |lib, cx| lib.set_up_vault(password, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(recovery) => {
                        this.recovery = Some(recovery);
                        this.recovery_saved = false;
                        for input in [this.master.clone(), this.confirm.clone()] {
                            input.update(cx, |i, cx| i.set_value("", window, cx));
                        }
                        this.acknowledged = false;
                        this.focus_primary(window, cx);
                    }
                    Err(err) => this.error = Some(format!("{err:#}").into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen(cx) == Screen::Link {
            self.link(window, cx);
            return;
        }
        let text = self.unlock.read(cx).value().to_string();
        if self.busy || text.is_empty() {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let task = self
            .library
            .update(cx, |lib, cx| lib.unlock_vault(SecretString::from(text), cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(true) => {
                        this.unlock.update(cx, |i, cx| i.set_value("", window, cx));
                        this.focus_primary(window, cx);
                    }
                    Ok(false) => {
                        this.error = Some("That password didn't unlock the vault.".into());
                        this.unlock.update(cx, |i, cx| {
                            i.focus(window, cx);
                            i.select_all(window, cx);
                        });
                    }
                    Err(err) => this.error = Some(format!("{err:#}").into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// "Use your Ottero account to unlock Passwords": the master password
    /// once, then the account opens the vault from now on.
    fn link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.unlock.read(cx).value().to_string();
        if self.busy || text.is_empty() {
            if text.is_empty() {
                self.error = Some("Enter your master password.".into());
                cx.notify();
            }
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let task = self.library.update(cx, |lib, cx| {
            lib.link_vault_to_account(SecretString::from(text), cx)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok((LinkResult::Linked, recovery)) => {
                        this.unlock.update(cx, |i, cx| i.set_value("", window, cx));
                        if let Some(key) = recovery {
                            this.recovery = Some(key);
                            this.recovery_saved = false;
                        }
                        window.push_notification(
                            "Passwords now opens with your Ottero account.",
                            cx,
                        );
                        this.focus_primary(window, cx);
                    }
                    Ok((LinkResult::WrongPassword, _)) => {
                        this.error = Some("That password didn't unlock the vault.".into());
                        this.unlock.update(cx, |i, cx| {
                            i.focus(window, cx);
                            i.select_all(window, cx);
                        });
                    }
                    Ok((LinkResult::Stale, _)) => {
                        this.error = Some(
                            "The account or the vault changed in the meantime. Nothing was changed; try again."
                                .into(),
                        );
                    }
                    Err(err) => this.error = Some(format!("{err:#}").into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Enter in the Ottero password field: unlock (and set up, on the
    /// setup screen).
    fn submit_account_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.screen(cx) {
            Screen::UnlockOttero => self.unlock_ottero(window, cx),
            Screen::SetupWithAccount => self.create_with_account(window, cx),
            _ => {}
        }
    }

    /// "Unlock Ottero": the account password opens the account key, and
    /// with it the linked vault (`Library::account_key_opened`).
    fn unlock_ottero(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.account_password.read(cx).value().to_string();
        if self.busy {
            return;
        }
        if text.is_empty() {
            self.error = Some("Enter your Ottero password.".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let task = self.library.update(cx, |lib, cx| {
            lib.unlock_account(SecretString::from(text), cx)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        this.account_password
                            .update(cx, |i, cx| i.set_value("", window, cx));
                        this.focus_primary(window, cx);
                    }
                    Err(message) => {
                        this.error = Some(message.into());
                        this.account_password.update(cx, |i, cx| {
                            i.focus(window, cx);
                            i.select_all(window, cx);
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Sets the vault up under the account (unlocking it first if needed).
    fn create_with_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let unlocked = self.library.read(cx).account().is_unlocked();
        let password = self.account_password.read(cx).value().to_string();
        if !unlocked && password.is_empty() {
            self.error = Some("Enter your Ottero password.".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let library = self.library.clone();
        let unlock = (!unlocked).then(|| {
            library.update(cx, |lib, cx| {
                lib.unlock_account(SecretString::from(password), cx)
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Some(unlock) = unlock
                && let Err(message) = unlock.await
            {
                _ = this.update_in(cx, |this, _, cx| {
                    this.busy = false;
                    this.error = Some(message.into());
                    cx.notify();
                });
                return;
            }
            let task = library.update(cx, |lib, cx| lib.set_up_vault_with_account(cx));
            let result = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        this.account_password
                            .update(cx, |i, cx| i.set_value("", window, cx));
                        this.recovery = None;
                        this.recovery_saved = false;
                        this.focus_primary(window, cx);
                    }
                    Err(err) => this.error = Some(format!("{err:#}").into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Fills both setup fields with a 5-word passphrase and shows it.
    fn suggest_passphrase(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use ottero_core::vault::generator::{Options, Style, generate};
        let options = Options {
            style: Style::Words,
            words: 5,
            ..Options::default()
        };
        let Some(phrase) = generate(&options, &mut crate::views::vault_item::random_below) else {
            self.error = Some(crate::views::vault_item::RANDOM_FAILED.into());
            cx.notify();
            return;
        };
        for input in [self.master.clone(), self.confirm.clone()] {
            input.update(cx, |i, cx| {
                i.set_value(phrase.to_string(), window, cx);
                i.set_masked(false, window, cx);
            });
        }
        cx.notify();
    }

    fn centered(content: impl IntoElement, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .p_6()
            .child(
                v_flex()
                    .w_full()
                    .max_w(rems(28.))
                    .p_8()
                    .gap_5()
                    .rounded(theme.radius_lg)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.group_box)
                    .when(theme.shadow, |s| s.shadow_xs())
                    .child(content),
            )
    }

    fn hero(
        icon: Lucide,
        title: &'static str,
        text: impl Into<SharedString>,
        cx: &App,
    ) -> impl IntoElement {
        let text: SharedString = text.into();
        let theme = cx.theme();
        v_flex()
            .items_center()
            .gap_3()
            .text_center()
            .child(
                div()
                    .size(rems(3.5))
                    .rounded_full()
                    .bg(theme.muted)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(icon).text_color(theme.foreground)),
            )
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(text),
            )
    }

    fn setup(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let estimate = self.master_estimate(cx);
        let problem = self.setup_problem(cx);
        let meter = estimate
            .as_ref()
            .map(|e| strength_meter(e.strength, Some(&e.crack_time), cx));
        let hint = estimate.as_ref().and_then(|e| e.hint.clone());
        let ack = self.acknowledged;
        Self::centered(
            v_flex()
                .gap_5()
                .child(Self::hero(
                    Lucide::KeyRound,
                    "Set up your vault",
                    "Passwords, cards and secure notes, encrypted on this device with a master password only you know.",
                    cx,
                ))
                .child(
                    v_flex()
                        .gap_2()
                        .child(Input::new(&self.master).id("vault-master").mask_toggle())
                        .children(meter)
                        .children(hint.map(|h| {
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(h)
                        }))
                        .child(Input::new(&self.confirm).id("vault-confirm").mask_toggle()),
                )
                .child(
                    Checkbox::new("vault-ack")
                        .checked(ack)
                        .label("I understand Ottero can't recover my vault if I forget this password.")
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.acknowledged = *checked;
                            cx.notify();
                        })),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            Button::new("vault-create")
                                .primary()
                                .w_full()
                                .label(if self.busy { "Securing your vault…" } else { "Create vault" })
                                .loading(self.busy)
                                .disabled(problem.is_some() || self.busy)
                                .on_click(cx.listener(|this, _, window, cx| this.create_vault(window, cx))),
                        )
                        .child(
                            div()
                                .h(rems(1.25))
                                .text_xs()
                                .text_center()
                                .text_color(if self.error.is_some() {
                                    theme.danger
                                } else {
                                    theme.muted_foreground
                                })
                                .child(
                                    self.error
                                        .clone()
                                        .or_else(|| problem.map(SharedString::from))
                                        .unwrap_or_else(|| "Ready.".into()),
                                ),
                        ),
                )
                .child(
                    h_flex().justify_center().child(
                        Button::new("vault-suggest")
                            .ghost()
                            .small()
                            .icon(Lucide::Sparkles)
                            .label("Suggest a passphrase")
                            .on_click(cx.listener(|this, _, window, cx| this.suggest_passphrase(window, cx))),
                    ),
                ),
            cx,
        )
        .into_any_element()
    }

    fn locked(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let busy = self.busy || self.library.read(cx).vault_state() == LockState::Unlocking;
        Self::centered(
            v_flex()
                .gap_5()
                .child(Self::hero(
                    Lucide::LockKeyhole,
                    "Vault locked",
                    "Enter your master password to unlock.",
                    cx,
                ))
                .child(
                    v_flex()
                        .gap_2()
                        .child(Input::new(&self.unlock).id("vault-unlock").mask_toggle())
                        .child(
                            Button::new("vault-unlock-button")
                                .primary()
                                .w_full()
                                .label(if busy { "Unlocking…" } else { "Unlock" })
                                .loading(busy)
                                .disabled(busy)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.unlock(window, cx)),
                                ),
                        )
                        .child(
                            div()
                                .h(rems(1.25))
                                .text_xs()
                                .text_center()
                                .text_color(theme.danger)
                                .children(self.error.clone()),
                        ),
                )
                .child(
                    h_flex().justify_center().child(
                        Button::new("vault-forgot")
                            .ghost()
                            .small()
                            .label("Forgot master password?")
                            .on_click(cx.listener(|this, _, window, cx| {
                                crate::views::vault_recovery::open_forgot(
                                    this.library.clone(),
                                    window,
                                    cx,
                                )
                            })),
                    ),
                ),
            cx,
        )
        .into_any_element()
    }

    /// The error line under a form: fixed height so the layout doesn't jump.
    fn error_line(&self, cx: &App) -> impl IntoElement {
        div()
            .h(rems(1.25))
            .text_xs()
            .text_center()
            .text_color(cx.theme().danger)
            .children(self.error.clone())
    }

    fn recovery_link(id: &'static str, label: &'static str, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .justify_center()
            .child(
                Button::new(id)
                    .ghost()
                    .small()
                    .label(label)
                    .on_click(cx.listener(|this, _, window, cx| {
                        crate::views::vault_recovery::open_forgot(this.library.clone(), window, cx)
                    })),
            )
            .into_any_element()
    }

    /// No vault yet, signed in: the account opens it, no master password.
    fn setup_with_account(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let lib = self.library.read(cx);
        let unlocked = lib.account().is_unlocked();
        let email = lib.account().signed_in_email().unwrap_or_default();
        let busy = self.busy;
        Self::centered(
            v_flex()
                .gap_5()
                .child(Self::hero(
                    Lucide::KeyRound,
                    "Set up Passwords",
                    format!("Passwords, cards and secure notes, encrypted on this device. Your Ottero account ({email}) unlocks them, so there's no extra password to remember."),
                    cx,
                ))
                .child(
                    v_flex()
                        .gap_2()
                        .when(!unlocked, |s| {
                            s.child(
                                Input::new(&self.account_password)
                                    .id("vault-account-password")
                                    .mask_toggle(),
                            )
                        })
                        .child(
                            Button::new("vault-create-account")
                                .primary()
                                .w_full()
                                .label(if busy { "Creating vault…" } else { "Create vault" })
                                .loading(busy)
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.create_with_account(window, cx)
                                })),
                        )
                        .child(self.error_line(cx)),
                ),
            cx,
        )
        .into_any_element()
    }

    /// A master-password vault while signed in: link it once.
    fn link_screen(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy || self.library.read(cx).vault_state() == LockState::Unlocking;
        let email = self
            .library
            .read(cx)
            .account()
            .signed_in_email()
            .unwrap_or_default();
        Self::centered(
            v_flex()
                .gap_5()
                .child(Self::hero(
                    Lucide::Cloud,
                    "Use your Ottero account",
                    format!("Enter your master password one last time to unlock Passwords with your Ottero account ({email}) from now on. The master password is then retired."),
                    cx,
                ))
                .child(
                    v_flex()
                        .gap_2()
                        .child(Input::new(&self.unlock).id("vault-link-master").mask_toggle())
                        .child(
                            Button::new("vault-link-button")
                                .primary()
                                .w_full()
                                .label(if busy { "Linking…" } else { "Unlock and link" })
                                .loading(busy)
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| this.link(window, cx))),
                        )
                        .child(self.error_line(cx)),
                )
                .child(Self::recovery_link("vault-forgot", "Forgot master password?", cx)),
            cx,
        )
        .into_any_element()
    }

    /// A linked vault that is locked: unlock Ottero, or explain why the
    /// account that is signed in can't open it.
    fn account_locked(&mut self, screen: &Screen, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy || self.library.read(cx).account().unlock_busy();
        let linked = self
            .library
            .read(cx)
            .vault_linked_email()
            .unwrap_or_default()
            .to_owned();
        let has_recovery = self.library.read(cx).vault_has_recovery();
        let fallback_label = if has_recovery {
            "Use recovery key…"
        } else {
            "Can't unlock? Start a new vault…"
        };
        let body = match screen {
            Screen::UnlockOttero => v_flex()
                .gap_5()
                .child(Self::hero(
                    Lucide::LockKeyhole,
                    "Unlock Ottero",
                    format!("Enter the password for {linked}. It opens your account and Passwords."),
                    cx,
                ))
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            Input::new(&self.account_password)
                                .id("vault-account-password")
                                .mask_toggle(),
                        )
                        .child(
                            Button::new("vault-account-unlock")
                                .primary()
                                .w_full()
                                .label(if busy { "Unlocking…" } else { "Unlock" })
                                .loading(busy)
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.unlock_ottero(window, cx)
                                })),
                        )
                        .child(self.error_line(cx)),
                )
                .child(Self::recovery_link("vault-forgot", fallback_label, cx)),
            Screen::SignedOut(email) => v_flex()
                .gap_5()
                .child(Self::hero(
                    Lucide::Cloud,
                    "Sign in to open Passwords",
                    format!("Passwords on this computer opens with your Ottero account, {email}."),
                    cx,
                ))
                .child(
                    Button::new("vault-sign-in")
                        .primary()
                        .w_full()
                        .label("Sign in…")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(crate::actions::SignInToCloud), cx)
                        }),
                )
                .child(Self::recovery_link("vault-forgot", fallback_label, cx)),
            Screen::OtherAccount(linked, current) => v_flex()
                .gap_5()
                .child(Self::hero(
                    Lucide::Cloud,
                    "Passwords belongs to another account",
                    if has_recovery {
                        format!("It opens with {linked}, and you're signed in as {current}. Nothing was changed. Sign in as {linked}, or use the recovery key.")
                    } else {
                        format!("It opens with {linked}, and you're signed in as {current}. Nothing was changed. Sign in as {linked} to open it.")
                    },
                    cx,
                ))
                .child(
                    Button::new("vault-other-account")
                        .w_full()
                        .label("Account settings…")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(crate::actions::OpenAccountSettings), cx)
                        }),
                )
                .child(Self::recovery_link("vault-forgot", fallback_label, cx)),
            _ => v_flex()
                .gap_5()
                .child(Self::hero(
                    Lucide::LockKeyhole,
                    "Your account can't open Passwords",
                    "This account's key doesn't open Passwords on this computer. It may have been linked to an earlier account with the same email, or its file was changed. Nothing was changed.",
                    cx,
                ))
                .child(Self::recovery_link("vault-forgot", fallback_label, cx)),
        };
        Self::centered(body, cx).into_any_element()
    }

    /// After setup (or "Create new recovery key"): show the key once.
    fn recovery_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let key = self
            .recovery
            .as_deref()
            .map(|k| k.to_string())
            .unwrap_or_default();
        let saved = self.recovery_saved;
        Self::centered(
            v_flex()
                .gap_5()
                .child(Self::hero(
                    Lucide::ShieldCheck,
                    "Save your recovery key",
                    if self.library.read(cx).vault_opener() == Some(Opener::Account) {
                        "If you can't sign in to your Ottero account, this key is the only other way into Passwords. Ottero shows it once."
                    } else {
                        "If you ever forget your master password, this key is the only way back into your vault. Ottero shows it once."
                    },
                    cx,
                ))
                .child(crate::views::vault_recovery::key_box(&key, cx))
                .child(
                    Checkbox::new("recovery-saved")
                        .checked(saved)
                        .label("I saved my recovery key somewhere safe, outside this computer.")
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.recovery_saved = *checked;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("recovery-done")
                        .primary()
                        .w_full()
                        .label("Continue")
                        .disabled(!saved)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.recovery = None;
                            this.recovery_saved = false;
                            this.focus_primary(window, cx);
                            cx.notify();
                        })),
                ),
            cx,
        )
        .into_any_element()
    }

    // ---- Unlocked ---------------------------------------------------------

    /// The "New login" command (only does something when unlocked).
    pub fn new_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.library.read(cx).vault_state() == LockState::Unlocked {
            self.new_item(ItemKind::Login, window, cx);
        } else {
            self.focus_primary(window, cx);
        }
    }

    fn new_item(&mut self, kind: ItemKind, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush_edit(window, cx) {
            return;
        }
        self.selected = None;
        self.pane.update(cx, |p, cx| p.start_new(kind, window, cx));
        cx.notify();
    }

    fn new_from_template(
        &mut self,
        template: Template,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.flush_edit(window, cx) {
            return;
        }
        self.selected = None;
        self.pane
            .update(cx, |p, cx| p.start_template(template, window, cx));
        cx.notify();
    }

    fn row(&self, item: &VaultItem, issues: &[&'static str], cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let id = item.id();
        let selected = self.selected == Some(id);
        let code = (self.filter == VaultFilter::Codes)
            .then(|| item.otp_setup().and_then(|s| totp::parse(s).ok()))
            .flatten()
            .map(|t| t.code_at(unix_now()));
        let subtitle: SharedString = match &code {
            Some(c) => format!("{} · {}s", c.grouped(), c.seconds_left).into(),
            None if issues.is_empty() => item.subtitle().into(),
            None => issues.join(" · ").into(),
        };
        h_flex()
            .id(SharedString::from(format!("vault-row-{id}")))
            .w_full()
            .h(rems(ROW_HEIGHT))
            .px_3()
            .gap_3()
            .items_center()
            .rounded(theme.radius)
            .border_1()
            .border_color(if selected {
                theme.border
            } else {
                gpui_kit::transparent_black()
            })
            .map(|s| {
                if selected {
                    s.bg(theme.group_box).when(theme.shadow, |s| s.shadow_xs())
                } else {
                    s.hover(|s| s.bg(theme.list_hover))
                }
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.list_focus, cx);
                this.select(Some(id), window, cx);
            }))
            .test_support()
            .child(avatar(item, rems(2.), cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(item.title().to_owned()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .when(code.is_some(), |s| {
                                s.font_family(theme.mono_font_family.clone())
                            })
                            .text_color(if issues.is_empty() {
                                theme.muted_foreground
                            } else {
                                theme.warning
                            })
                            .child(subtitle),
                    ),
            )
            .when(item.is_favorite(), |s| {
                s.child(
                    Icon::new(Lucide::Star)
                        .xsmall()
                        .text_color(theme.muted_foreground),
                )
            })
            .into_any_element()
    }

    fn list_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let health = (self.filter == VaultFilter::Health)
            .then(|| self.library.update(cx, |lib, _| lib.vault_health()));
        let rows: Vec<Row> =
            self.rows[range.start.min(self.rows.len())..range.end.min(self.rows.len())].to_vec();
        let theme = cx.theme().clone();
        rows.into_iter()
            .map(|row| match row {
                Row::Header(text) => div()
                    .h(rems(HEADER_HEIGHT))
                    .px_3()
                    .pt_2()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.muted_foreground)
                    .child(text)
                    .into_any_element(),
                Row::Item(id) => match self.library.read(cx).vault_item(id) {
                    Some(item) => {
                        let issues = health
                            .as_ref()
                            .map(|h| h.issues_for(id))
                            .unwrap_or_default();
                        self.row(&item, &issues, cx)
                    }
                    None => div().h(rems(ROW_HEIGHT)).into_any_element(),
                },
            })
            .collect()
    }

    fn filter_title(&self) -> &'static str {
        match self.filter {
            VaultFilter::All => "All items",
            VaultFilter::Favorites => "Favorites",
            VaultFilter::Kind(k) => k.plural(),
            VaultFilter::Codes => "One-time codes",
            VaultFilter::Health => "Password health",
            VaultFilter::Trash => "Trash",
        }
    }

    fn list_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let count = self.ids.len();
        let view = cx.entity().downgrade();
        let new_menu = Button::new("vault-new")
            .small()
            .icon(Lucide::Plus)
            .tooltip("New item")
            .dropdown_menu(move |menu, window, cx| {
                let item = |template: Template, view: gpui_kit::WeakEntity<Self>| {
                    PopupMenuItem::new(template.label())
                        .icon(template_icon(template))
                        .on_click(move |_, window, cx| {
                            _ = view.update(cx, |this, cx| {
                                this.new_from_template(template, window, cx)
                            });
                        })
                };
                let more_view = view.clone();
                let menu = Template::PRIMARY
                    .into_iter()
                    .fold(menu, |menu, t| menu.item(item(t, view.clone())));
                menu.separator()
                    .submenu("More", window, cx, move |sub, _, _| {
                        Template::MORE
                            .into_iter()
                            .fold(sub, |sub, t| sub.item(item(t, more_view.clone())))
                    })
            });
        let sort = vault_prefs::get(cx).sort;
        let view = cx.entity().downgrade();
        let more = Button::new("vault-more")
            .ghost()
            .small()
            .icon(Lucide::Ellipsis)
            .tooltip("Vault")
            .dropdown_menu(move |menu, window, cx| {
                let gen_view = view.clone();
                let import_view = view.clone();
                let sort_view = view.clone();
                menu.submenu("Sort by", window, cx, move |sub, _, _| {
                    VaultSort::ALL.into_iter().fold(sub, |sub, s| {
                        let view = sort_view.clone();
                        sub.item(PopupMenuItem::new(s.label()).checked(s == sort).on_click(
                            move |_, window, cx| {
                                vault_prefs::update(cx, |p| p.sort = s);
                                _ = view.update(cx, |this, cx| this.refresh(window, cx));
                            },
                        ))
                    })
                })
                .separator()
                .item(
                    PopupMenuItem::new("Generate password…")
                        .icon(Lucide::Dices)
                        .on_click(move |_, window, cx| {
                            _ = gen_view.update(cx, |this, cx| {
                                crate::views::vault_generator::open_generator(
                                    None,
                                    &this.library,
                                    window,
                                    cx,
                                )
                            });
                        }),
                )
                .item(
                    PopupMenuItem::new("Import from CSV…")
                        .icon(Lucide::FolderInput)
                        .on_click(move |_, window, cx| {
                            _ = import_view.update(cx, |this, cx| this.import(window, cx));
                        }),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Lock vault")
                        .icon(Lucide::LockKeyhole)
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(crate::actions::LockVault), cx)
                        }),
                )
            });
        let rem = window.rem_size();
        let sizes: std::rc::Rc<Vec<gpui_kit::Size<gpui_kit::Pixels>>> = std::rc::Rc::new(
            self.rows
                .iter()
                .map(|r| {
                    let h = match r {
                        Row::Header(_) => HEADER_HEIGHT,
                        Row::Item(_) => ROW_HEIGHT,
                    };
                    gpui_kit::size(gpui_kit::px(0.), rem * h)
                })
                .collect(),
        );
        let list = v_virtual_list(
            cx.entity(),
            "vault-list",
            sizes,
            |this, range: Range<usize>, _, cx| this.list_rows(range, cx),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0()
        .px_2()
        .pb_2();
        let empty_text: Option<(&'static str, &'static str)> = (count == 0).then(|| {
            let searching = !self.search.read(cx).value().is_empty();
            match (searching, self.filter) {
                (true, _) => ("No matches", "Try another word, or clear the search."),
                (_, VaultFilter::Trash) => (
                    "Trash is empty",
                    "Items you delete stay here until you empty the trash.",
                ),
                (_, VaultFilter::Favorites) => ("No favorites", "Star an item to keep it here."),
                (_, VaultFilter::Health) => ("All clear", "No weak, reused or old passwords."),
                (_, VaultFilter::Codes) => (
                    "No one-time codes",
                    "Add a setup key to a login, or New → More → Authenticator.",
                ),
                _ => (
                    "Nothing here yet",
                    "Add a login with New, or import from another app.",
                ),
            }
        });
        v_flex()
            .w(rems(22.))
            .flex_shrink_0()
            .h_full()
            .border_r_1()
            .border_color(theme.border)
            .child(
                v_flex()
                    .px_3()
                    .pt_4()
                    .pb_2()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .on_key_down(cx.listener(
                                        |this, e: &KeyDownEvent, window, cx| {
                                            // ↓ from the search field moves into the list.
                                            if e.keystroke.key == "down" && !this.ids.is_empty() {
                                                cx.stop_propagation();
                                                window.focus(&this.list_focus, cx);
                                                this.step(1, window, cx);
                                            }
                                        },
                                    ))
                                    .child(
                                        Input::new(&self.search)
                                            .id("vault-search")
                                            .small()
                                            .cleanable(true)
                                            .prefix(
                                                Icon::new(Lucide::Search)
                                                    .xsmall()
                                                    .text_color(theme.muted_foreground),
                                            ),
                                    ),
                            )
                            .child(new_menu)
                            .child(more),
                    )
                    .child(
                        h_flex()
                            .px_1()
                            .h(rems(1.5))
                            .justify_between()
                            .items_center()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .map(|row| match self.tag.clone() {
                                // A tag shows as a removable chip in the title's place.
                                Some(tag) => row.child(
                                    Button::new("vault-clear-tag")
                                        .xsmall()
                                        .outline()
                                        .label(format!("#{tag}"))
                                        .icon(Lucide::X)
                                        .tooltip("Show all items")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.set_tag(None, window, cx)
                                        })),
                                ),
                                None => row.child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(self.filter_title()),
                                ),
                            })
                            .child(count.to_string()),
                    ),
            )
            .child(
                div()
                    .id("vault-list-focus")
                    .track_focus(&self.list_focus)
                    .key_context(VAULT_CONTEXT)
                    .on_action(cx.listener(|this, _: &VaultPreviousItem, window, cx| {
                        this.step(-1, window, cx)
                    }))
                    .on_action(
                        cx.listener(|this, _: &VaultNextItem, window, cx| this.step(1, window, cx)),
                    )
                    .on_action(cx.listener(|this, _: &VaultCopyPassword, window, cx| {
                        this.copy_selected(CopyWhat::Password, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &VaultCopyUsername, window, cx| {
                        this.copy_selected(CopyWhat::Username, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &VaultCopyCode, window, cx| {
                        this.copy_selected(CopyWhat::Code, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &VaultEditSelected, window, cx| {
                        this.pane.update(cx, |p, cx| p.edit(window, cx));
                    }))
                    .on_action(cx.listener(|this, _: &VaultTrashSelected, window, cx| {
                        this.pane.update(cx, |p, cx| p.trash(window, cx));
                    }))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .map(|s| match empty_text {
                        Some((title, desc)) => {
                            s.child(empty_state(Lucide::KeyRound, title, desc, cx))
                        }
                        None => s.child(list),
                    }),
            )
            .into_any_element()
    }

    fn copy_selected(&mut self, what: CopyWhat, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self
            .selected
            .and_then(|id| self.library.read(cx).vault_item(id))
        else {
            return;
        };
        let f = item.fields();
        let (label, value) = match (what, f.kind) {
            (CopyWhat::Password, ItemKind::Card) => ("Card number", f.number.clone()),
            (CopyWhat::Password, _) => ("Password", f.password.clone()),
            (CopyWhat::Username, ItemKind::Card) => ("Cardholder", f.cardholder.clone()),
            (CopyWhat::Username, _) => ("Username", f.username.clone()),
            (CopyWhat::Code, _) => {
                let code = item
                    .otp_setup()
                    .and_then(|s| ottero_core::vault::totp::parse(s).ok())
                    .map(|t| t.code_at(unix_now()).code);
                ("One-time code", code.unwrap_or_default())
            }
        };
        if value.is_empty() {
            ottero_ui::feedback::error_toast(
                format!("This item has no {}.", label.to_lowercase()),
                window,
                cx,
            );
            return;
        }
        copy_field(
            label,
            value,
            label != "Username" && label != "Cardholder",
            window,
            cx,
        );
        let id = item.id();
        self.library
            .update(cx, |lib, cx| lib.mark_vault_used(id, cx));
    }

    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import".into()),
        });
        let library = self.library.clone();
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let text = cx
                .background_executor()
                .spawn(async move { std::fs::read_to_string(path) })
                .await;
            _ = cx.update(|window, cx| {
                let text = match text {
                    Ok(t) => ottero_crypto::Zeroizing::new(t),
                    Err(_) => {
                        ottero_ui::feedback::error_toast("Couldn't read that file.", window, cx);
                        return;
                    }
                };
                match ottero_core::vault::import::parse_csv(&text) {
                    Err(msg) => ottero_ui::feedback::error_toast(msg, window, cx),
                    Ok(parsed) => {
                        let skipped = parsed.skipped;
                        let source = parsed.source.map_or("CSV", |s| s.label());
                        let result =
                            library.update(cx, |lib, cx| lib.import_vault_items(parsed.items, cx));
                        if let Some(n) = ottero_ui::feedback::report(result, window, cx) {
                            let mut msg = format!(
                                "Imported {n} item{} from {source}.",
                                if n == 1 { "" } else { "s" }
                            );
                            if skipped > 0 {
                                msg.push_str(&format!(
                                    " Skipped {skipped} empty row{}.",
                                    if skipped == 1 { "" } else { "s" }
                                ));
                            }
                            msg.push_str(" Delete the CSV file: it isn't encrypted.");
                            window.push_notification(msg, cx);
                        }
                    }
                }
            });
        })
        .detach();
    }

    fn unlocked(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .size_full()
            .child(self.list_pane(window, cx))
            .child(div().flex_1().min_w_0().h_full().child(self.pane.clone()))
            .into_any_element()
    }
}

#[derive(Clone, Copy)]
enum CopyWhat {
    Password,
    Username,
    Code,
}

/// Seconds since the Unix epoch (for one-time codes).
pub fn unix_now() -> u64 {
    u64::try_from(ottero_core::clock::now().as_second()).unwrap_or(0)
}

pub fn template_icon(t: Template) -> Lucide {
    match t {
        Template::Login => Lucide::KeyRound,
        Template::Card => Lucide::CreditCard,
        Template::Note => Lucide::StickyNote,
        Template::Wifi => Lucide::Wifi,
        Template::License => Lucide::BadgeCheck,
        Template::ApiKey | Template::Environment => Lucide::Code,
        Template::SshKey => Lucide::Terminal,
        Template::Identity => Lucide::IdCard,
        Template::Authenticator => Lucide::ShieldCheck,
    }
}

pub fn kind_icon(kind: ItemKind) -> Lucide {
    match kind {
        ItemKind::Login => Lucide::KeyRound,
        ItemKind::Card => Lucide::CreditCard,
        ItemKind::Note => Lucide::StickyNote,
    }
}

/// Four segments that fill with strength, the label, and an optional crack time.
pub fn strength_meter(strength: Strength, crack_time: Option<&str>, cx: &App) -> AnyElement {
    let crack_time = crack_time.map(str::to_owned);
    let theme = cx.theme();
    let level = strength.level();
    let color = match strength {
        Strength::VeryWeak | Strength::Weak => theme.danger,
        Strength::Fair => theme.warning,
        Strength::Strong | Strength::VeryStrong => theme.success,
    };
    v_flex()
        .gap_1()
        .child(h_flex().gap_1().children((1..=4).map(|i| {
            div()
                .flex_1()
                .h(rems(0.25))
                .rounded_full()
                .bg(if level >= i { color } else { theme.muted })
        })))
        .child(
            h_flex()
                .justify_between()
                .text_xs()
                .child(div().text_color(color).child(strength.label()))
                .children(crack_time.map(|t| {
                    div()
                        .text_color(theme.muted_foreground)
                        .child(format!("Time to crack: {t}"))
                })),
        )
        .into_any_element()
}

impl Focusable for PasswordsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PasswordsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.library.read(cx).vault_state();
        if state != LockState::Unlocked && self.recovery.is_some() {
            // Never leave a recovery key on screen once the vault locks.
            self.recovery = None;
        }
        let body = match self.screen(cx) {
            Screen::Setup => self.setup(cx),
            Screen::SetupWithAccount => self.setup_with_account(cx),
            Screen::Locked => self.locked(cx),
            Screen::Link => self.link_screen(cx),
            screen @ (Screen::UnlockOttero
            | Screen::SignedOut(_)
            | Screen::OtherAccount(..)
            | Screen::WrongKey) => self.account_locked(&screen, cx),
            Screen::Recovery => self.recovery_step(cx),
            Screen::Unlocked => self.unlocked(window, cx),
        };
        div()
            .id("passwords")
            .track_focus(&self.focus)
            .size_full()
            .child(body)
    }
}

/// Opens "Change master password". Only while unlocked.
pub fn open_change_password(library: Entity<Library>, window: &mut Window, cx: &mut App) {
    if library.read(cx).vault_opener() == Some(Opener::Account) {
        ottero_ui::feedback::error_toast(
            "Passwords opens with your Ottero account, so it has no master password.",
            window,
            cx,
        );
        return;
    }
    if library.read(cx).vault_state() != LockState::Unlocked {
        ottero_ui::feedback::error_toast("Unlock the vault first.", window, cx);
        return;
    }
    let form = cx.new(|cx| ChangePassword::new(library, window, cx));
    let shown = form.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Change master password")
            .w(gpui_kit::px(440.))
            // Enter belongs to the form's fields (they submit themselves);
            // it must never just close the dialog and drop what was typed.
            .on_ok(|_, _, _| false)
            .child(shown.clone())
    });
    // Opening the dialog focuses the dialog itself; the first field goes first.
    let current = form.read(cx).current.clone();
    current.update(cx, |i, cx| i.focus(window, cx));
}

pub struct ChangePassword {
    library: Entity<Library>,
    current: Entity<InputState>,
    new: Entity<InputState>,
    confirm: Entity<InputState>,
    error: Option<SharedString>,
    busy: bool,
    _subscriptions: Vec<Subscription>,
}

impl ChangePassword {
    fn new(library: Entity<Library>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let masked = |p: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(p))
        };
        let current = masked("Current master password", window, cx);
        let new = masked("New master password", window, cx);
        let confirm = masked("Type the new one again", window, cx);
        let subscriptions = [&current, &new, &confirm]
            .into_iter()
            .map(|input| {
                cx.subscribe_in(input, window, |this, _, e, window, cx| match e {
                    InputEvent::Change => {
                        this.error = None;
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => this.submit(window, cx),
                    _ => {}
                })
            })
            .collect();
        Self {
            library,
            current,
            new,
            confirm,
            error: None,
            busy: false,
            _subscriptions: subscriptions,
        }
    }

    fn problem(&self, cx: &App) -> Option<&'static str> {
        let current = self.current.read(cx).value();
        let new = self.new.read(cx).value();
        let confirm = self.confirm.read(cx).value();
        if current.is_empty() {
            return Some("Enter your current master password.");
        }
        if new.chars().count() < MIN_MASTER_CHARS {
            return Some("Use at least 10 characters for the new one.");
        }
        if estimate(&new, &[]).strength < Strength::Fair {
            return Some("Choose something harder to guess.");
        }
        if new != confirm {
            return Some("The new passwords don't match.");
        }
        None
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.problem(cx).is_some() {
            return;
        }
        self.busy = true;
        cx.notify();
        let current = SecretString::from(self.current.read(cx).value().to_string());
        let new = SecretString::from(self.new.read(cx).value().to_string());
        let task = self
            .library
            .update(cx, |lib, cx| lib.change_master_password(current, new, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(true) => {
                        window.close_dialog(cx);
                        window.push_notification("Master password changed.", cx);
                    }
                    Ok(false) => this.error = Some("The current password isn't right.".into()),
                    Err(err) => this.error = Some(format!("{err:#}").into()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for ChangePassword {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let new = self.new.read(cx).value().to_string();
        let meter = (!new.is_empty()).then(|| {
            let e = estimate(&new, &[]);
            strength_meter(e.strength, Some(&e.crack_time), cx)
        });
        let problem = self.problem(cx);
        v_flex()
            .gap_4()
            .child(Input::new(&self.current).mask_toggle())
            .child(
                v_flex()
                    .gap_2()
                    .child(Input::new(&self.new).mask_toggle())
                    .children(meter)
                    .child(Input::new(&self.confirm).mask_toggle()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(if self.error.is_some() { theme.danger } else { theme.muted_foreground })
                    .child(
                        self.error
                            .clone()
                            .or_else(|| problem.map(SharedString::from))
                            .unwrap_or_else(|| "Your items stay as they are; only the key that protects them changes.".into()),
                    ),
            )
            .child(
                h_flex().justify_end().child(
                    Button::new("change-master")
                        .primary()
                        .label(if self.busy { "Changing…" } else { "Change password" })
                        .loading(self.busy)
                        .disabled(problem.is_some() || self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                ),
            )
    }
}

/// Opens "Use your Ottero account for Passwords" for an unlocked
/// master-password vault: the master password once, then the account opens
/// it. A recovery key made on the way is shown by `passwords`.
pub fn open_link_dialog(
    library: Entity<Library>,
    passwords: gpui_kit::WeakEntity<PasswordsView>,
    window: &mut Window,
    cx: &mut App,
) {
    let form = cx.new(|cx| LinkVault::new(library, passwords, window, cx));
    let shown = form.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Use your Ottero account for Passwords")
            .w(gpui_kit::px(440.))
            // Enter belongs to the form's fields (they submit themselves);
            // it must never just close the dialog and drop what was typed.
            .on_ok(|_, _, _| false)
            .child(shown.clone())
    });
    // Opening the dialog focuses the dialog itself; the field comes first.
    let master = form.read(cx).master.clone();
    master.update(cx, |i, cx| i.focus(window, cx));
}

pub struct LinkVault {
    library: Entity<Library>,
    passwords: gpui_kit::WeakEntity<PasswordsView>,
    master: Entity<InputState>,
    error: Option<SharedString>,
    busy: bool,
    _subscriptions: Vec<Subscription>,
}

impl LinkVault {
    fn new(
        library: Entity<Library>,
        passwords: gpui_kit::WeakEntity<PasswordsView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let master = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Master password")
        });
        let sub = cx.subscribe_in(&master, window, |this, _, e, window, cx| match e {
            InputEvent::Change => {
                this.error = None;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => this.submit(window, cx),
            _ => {}
        });
        Self {
            library,
            passwords,
            master,
            error: None,
            busy: false,
            _subscriptions: vec![sub],
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.master.read(cx).value().to_string();
        if self.busy {
            return;
        }
        if text.is_empty() {
            self.error = Some("Enter your master password.".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let task = self.library.update(cx, |lib, cx| {
            lib.link_vault_to_account(SecretString::from(text), cx)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok((LinkResult::Linked, recovery)) => {
                        window.close_dialog(cx);
                        window.push_notification(
                            "Passwords now opens with your Ottero account.",
                            cx,
                        );
                        if let Some(key) = recovery {
                            _ = this
                                .passwords
                                .update(cx, |p, cx| p.show_recovery_key(key, cx));
                        }
                    }
                    Ok((LinkResult::WrongPassword, _)) => {
                        this.error = Some("That password didn't unlock the vault.".into());
                    }
                    Ok((LinkResult::Stale, _)) => {
                        this.error = Some(
                            "The account or the vault changed in the meantime. Nothing was changed; try again."
                                .into(),
                        );
                    }
                    Err(err) => this.error = Some(format!("{err:#}").into()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for LinkVault {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let email = self
            .library
            .read(cx)
            .account()
            .signed_in_email()
            .unwrap_or_default();
        v_flex()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!("Enter your master password one last time. After that, your Ottero password ({email}) opens Passwords, and the master password is retired. Your items stay as they are.")),
            )
            .child(Input::new(&self.master).id("link-master").mask_toggle())
            .child(
                div()
                    .h(rems(1.25))
                    .text_xs()
                    .text_color(theme.danger)
                    .children(self.error.clone()),
            )
            .child(
                h_flex().justify_end().child(
                    Button::new("link-vault")
                        .primary()
                        .label(if self.busy { "Linking…" } else { "Link" })
                        .loading(self.busy)
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                ),
            )
    }
}
