# Architecture

## Layers

```
apps/desktop (GPUI shell, screens, actions)      ← later: apps/mobile, apps/web, server/
   │ uses
   ├── crates/ui       design system (gpui-kit only)
   ├── crates/storage  SQLite + migrations + FTS5
   ├── crates/crypto   vault crypto (no UI, no IO)
   └── crates/core     domain types and rules (no IO)
```

Only `apps/*` and `crates/ui` know about GPUI. Everything a future mobile or web
app needs (rules, storage, crypto) is UI-free, so a new platform is a new shell.

## Data flow

```
user input → View (Entity) → Library method → core rule → Store (SQLite)
                                   │
                                   └─ emit LibraryEvent ──▶ subscribed views reload
```

- `Library` (`apps/desktop/src/library.rs`) is the single owner of the `Store`.
- Views keep their own view state (selection, input entities, tab) and re-query
  on `LibraryEvent`. No view caches another view's data.
- Writes are synchronous single transactions (sub-millisecond in WAL mode). If a
  feature needs slow work (Argon2, indexing, sync), run it with
  `cx.background_spawn` and apply the result on the foreground.

## Startup (`ottero_desktop::start`)

1. Resolve data folder (`paths.rs`) and open/migrate SQLite (fails fast, before the window).
2. `gpui_kit::init`, then `ottero_ui::init` (fonts → themes → saved appearance).
3. Bind keys, set menus, register app-level command handlers.
4. Open the window with `AppShell`; register it as the command target.

UI tests call the same `start`, so they exercise the real startup.

## Commands

Every command is a GPUI action. `shell::register_commands` installs app-level
handlers that forward to the shell through `with_shell`, which **defers** the
update (the window is borrowed during dispatch). Shortcuts, the macOS menu bar
and the command palette therefore run identical code.

## State machines

| Thing | Where | Pinned by |
| --- | --- | --- |
| Task status (Todo/InProgress/Done/Cancelled) | `crates/core/src/task.rs` | `transition_table_is_exact` |
| Trash (active ⇄ trashed) for notes and tasks | `note.rs`, `task.rs` | `trashed_*` tests |
| Note editor (Empty/Clean/Dirty, debounced autosave, flush on switch/blur) | `views/notes.rs` | `switching_notes_flushes_unsaved_typing`, `new_note_typing_autosaves_with_links` |
| Appearance (load → sanitize → apply; System follows OS) | `crates/ui/src/appearance.rs` | appearance tests, `theme_commands_switch_mode_and_save` |
| Sidebar layout (width clamp, collapsed, debounced save) | `ui_state.rs`, `shell.rs` | `sidebar_toggle_persists_layout` |

## Files on disk

Data folder (`Settings → General → Data folder`):

- `ottero.sqlite3` (+ `-wal`, `-shm`): notes, tasks, events, habits, and the vault (ciphertext only).
- `vault.json`: vault preferences (auto-lock, clipboard); `vault-archives/*.otvault` (older `.cfvault`/`.ksvault` still open): archived vaults.
- `appearance.json`: appearance settings.
- `layout.json`: sidebar width/collapsed.
- Its `navigation` object stores stable catalog IDs for sidebar order,
  visibility and startup. Initialize it once before first-launch defaults are
  written; an existing database or preferences preserves legacy navigation,
  while fresh installations get a smaller preset. One LayoutStore owns writes.
  Appearance reset cannot reset workspace choices. Unknown or repeated IDs
  are resolved safely; missing/new catalog entries remain accessible in More.

Debug builds use a separate `Ottero Dev` folder; `OTTERO_DATA_DIR` overrides.
A Keepstone folder from before the rename is moved here on first launch (`paths.rs`).

Developer extends existing task projects with `developer_projects` metadata
(migration 0011), rather than creating a second project system. Setup notes
are real Notes; archive hides the workspace without deleting linked data.
Environment sets are optional structured fields inside existing encrypted vault
items (item-data v2, ordinary items stay v1). Developer and Passwords share the
same masked editor; the encryption envelope and KDF have not changed. File IO
goes through Library, and late file-dialog results are invalidated by draft
generations. See [the feature plan](plans/11-developer.md) and
[security policy](security.md#developer-projects-and-environment-secrets).

## Sync (optional Ottero Cloud)

Local-first stays the source of truth. The optional Ottero Cloud sync server
stores only end-to-end encrypted change blobs (`crates/sync-protocol`).
The schema uses UUID v7 IDs, `updated_at`, and soft deletes for this.
See `docs/plans/06-sync.md`.

## Native updates

`updates` owns scheduling, signed catalog validation, preferences and GPUI status.
macOS delegates to a UI-free Swift SPUUserDriver in a nested helper app and the
core-only Sparkle framework. Windows delegates to Velopack with a signed catalog
UpdateSource. Native engines own extraction/replacement/relaunch; the app owns
draft guards and a SQLite online-backup snapshot before explicit restart. Neither
engine changes the vault encryption format. See ADR 0008 for packaging boundaries.
