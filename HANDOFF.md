# HANDOFF

Overwrite **Current state** and **Next steps** every session. Append to the log.

## Current state (2026-11-26)

- App: **Ottero by Nexdark Labs** (formerly Calmfold, formerly Keepstone).
  Branch `main`, repo `MohammedSafwan10/ottero`. Version **0.4.2** (tagged `v0.4.2`).
  `scripts/check.sh` passes across all workspace crates (including `ottero-sync-client`,
  `ottero-mobile-ffi`, and the 2-device E2EE sync integration test `two_device_sync.rs`).
  - **Shared E2EE Sync Crate (`crates/sync-client`):**
    - Extracted Desktop's sync client, engine, and canonical mutation helpers
      (`derive_sync_account_id`, `enqueue_note_change`, `enqueue_note_tombstone`,
      `enqueue_task_change`, `enqueue_event_change`, `enqueue_habit_change`,
      `enqueue_vault_item_change`, `apply_decrypted_page`, `sync_once_with_apply`)
      into shared workspace crate `ottero-sync-client` used identically by both
      Desktop and Android.
    - Fixed Desktop `empty_note_trash`, `empty_task_trash`, `empty_event_trash`,
      `add_subtask`, `delete_subtask`, `open_daily_note`, and `import_notes` to
      emit sync changes/tombstones across all devices (`v0.4.2`).
    - **Low-Latency Live Sync & Quiet Unlock UX (next release):**
      - Persistent keep-alive `SyncClient::shared` pool (eliminates per-sync TLS handshakes).
      - Local collection-key cache fast-path (skips redundant `GET /v1/sync/keys` when all 8 collection keys are cached).
      - Immediate (`0 ms`) push for discrete mutations (tasks, habits, events, vault, trash) while keeping `350 ms` debounce for continuous note typing.
      - Backward-compatible inline encrypted WebSocket push (`WsServerMessage::SeqAdvanced { latest_seq, changes }`, `apply_inline_remote_changes` on Desktop and `apply_ws_frame` on Android) for single-hop sub-200 ms live delivery with automatic HTTP `/pull` fallback on sequence gaps.
      - Quiet Desktop sidebar footer action (`Unlock sync` / `Sync issue`) shown only when Cloud is signed in and locked or errored.
    - **Android Verified In-App Updates (`direct` vs `play` flavors):**
      - `scripts/release-metadata.py` includes signed `android-apk` (`url`, `sha256`, `size`) in `stable.json` + `stable.json.minisig`.
      - `crates/mobile-ffi/src/update.rs` verifies `stable.json` against `ottero_core::update::RELEASE_KEY`.
      - `apps/android/app/build.gradle.kts` defines `direct` (`SELF_UPDATE_ENABLED=true`, `REQUEST_INSTALL_PACKAGES` + `FileProvider` in `src/direct/`) and `play` (`SELF_UPDATE_ENABLED=false`, Play-compliant) flavors, with `AppUpdater.kt` streaming SHA-256 verification and Settings → About **Check now** UI.
  - **Native Android App (`crates/mobile-ffi` + `apps/android`, Phases 0–6 + Parity Waves 1–4):**
    - Full UniFFI bridge (`crates/mobile-ffi`: `notes`, `tasks`, `calendar`,
      `habits`, `focus`, `today`, `vault`, `sync`) sharing `ottero-core`,
      `ottero-storage`, `ottero-crypto`, and `ottero-sync-client`.
    - Cross-compilation script (`scripts/build-android-rust.sh`) building `arm64-v8a`
      + `x86_64` `libottero_mobile_ffi.so` via `cargo-ndk` (NDK 28, 16 KB page-size
      ready) and generating Kotlin UniFFI bindings from the unstripped host cdylib.
    - Jetpack Compose app (`apps/android`, AGP 9 / Gradle 9.1.0 / `compileSdk = 36`):
      - **Navigation & UX:** Quiet Alabaster & Smoked Obsidian themes, Quiet Loop
        adaptive icon + Android 12+ splash screens, onboarding, hybrid 4-tab bottom
        bar (`Today`, `Notes`, `Tasks`, `Habits`) + drawer/rail (`Calendar`, `Focus`,
        `Passwords`, `Trash`), `QuickCaptureSheet`, `OtteroPullToRefresh`, and
        scalable **Settings Hub + Sub-Pages + Tablet Split-Pane**.
      - **Realtime E2EE Sync:** Account sign-in + 2FA TOTP/backup codes, hardware
        `AndroidKeyStore` session persistence, 500 ms debounced auto-sync,
        foreground OkHttp WebSocket listener (`/v1/sync/ws`, `HTTP 101` verified),
        network-recovery reconnect, offline account-linked vault re-unlock, and
        live in-place refresh in `NoteEditorScreen`.
      - **Modules:** Notes (Markdown toolbar, `#tag` filters, live refresh), Tasks
        (swipe-to-complete/trash with fixed Undo, `TaskDetailSheet`, Projects,
        Subtasks, recurrence), Calendar (Week/Month toggle, `EventDetailSheet`,
        `ottero_core::holidays`, 12h/24h & week-start prefs), Habits (7-day history
        dots, streaks, `HabitDetailSheet` edit/archive/delete), Focus (wall-clock
        timer persisted & synced via `Collection::Focus` + today log), Passwords
        (Logins/Cards/Secure Notes, Favorites, TOTP, generator, `FLAG_SECURE`,
        `BiometricPrompt`, configurable clipboard auto-clear), and Unified Trash
        (Notes/Tasks/Events + Empty Trash).
      - **Android Native & Release CI:** `ACTION_SEND` Share Sheet ("Save to Ottero"),
        `OtteroAutofillService` (`BIND_AUTOFILL_SERVICE`), hardened R8/ProGuard
        `app-release.apk` (18 MB, verified live on Android 16 hardware), and
        automated `android` job in `.github/workflows/release.yml`.
  - **Live Cross-Device Note Sync (`0.4.1`):** Open notes live-refresh in place
    when remote sync updates arrive (`refresh_open_note` with `reloading` guard),
    and `AUTO_SYNC_DEBOUNCE` tightened from `1.5 s` to `500 ms`.
  - **Complete Quiet Alabaster & Smoked Obsidian Redesign (`0.4.0`):** Elevated
    surfaces across Focus, Calendar, Notes, Passwords, and Developer; responsive
    wrapping headers; refreshed README/docs/Today screenshot; automated
    `publish` job in `.github/workflows/release.yml` for `ottero-releases` (renamed from `calmfold-releases`).
  - **Responsive Settings & Calmer Notifications (`0.3.1`):** Stacked wide Settings
    rows (`Updates`, `Interface`/`Code` fonts, `Line spacing`, `Content width`,
    `Lock after`, `Unlock with`) and wrapped Account action rows so buttons like
    `Restart to update` never clip on narrower windows; capped visible toasts to 3
    at `340 px` width and deduplicated update, clipboard, error, undo, and info toasts.
  *(Note: keep `HANDOFF.md` strictly about the open-source desktop app; cloud
  backend, hosting, and pricing/monetization notes belong in the private
  `ottero-cloud` repo — see `AGENTS.md` rule 13.)*
- **Desktop E2EE Sync client (implemented + audited):**
  - Shared IO-free protocol crate `crates/sync-protocol` (`Hlc`/`HlcClock`,
    8 collections, separator-safe `item_aad` authenticating `hlc` & `deleted`,
    wire DTOs, 1 MiB blob / 200-item / 8 MiB batch limits, WS notices).
  - Crypto + storage: `crates/crypto/src/sync.rs` (HKDF-SHA256 per-collection
    key wrapping + AEAD sealing), migration `0013_sync.sql`, `crates/storage/src/sync.rs`
    (`sync_state`, `sync_collection_keys`, `sync_outbox`, strict per-field LWW
    `sync_field_clocks`), O(1) `Store::vault_item(id)`.
  - Desktop sync engine (`apps/desktop/src/sync/`): `SyncClient`, `sync_once_with_apply`
    (adopts server-authoritative keys on `PUT /v1/sync/keys` race, byte-budgeted
    push batching, FK-ordered `apply_decrypted_page` before persisting `last_pull_seq`,
    `Zeroizing<String>` payloads, `apply_task` tombstone resurrection guard &
    `recurrence` sync, multi-device `vault-meta` sync + automatic unlock/reload
    on fresh devices), automatic sync (sign-in/unlock trigger, 1.5 s debounced
    local writes, 60 s background fallback timer, foreground focus sync, and
    TLS WebSocket live push listener `/v1/sync/ws`), plus non-blocking `SyncNow`
    & hosted billing portal actions in Settings → Account.
  - Cloud Passwords UX (migration `0014_vault_account_optional_recovery.sql`):
    Cloud-linked vaults no longer force a redundant second recovery key screen
    (Account Emergency Kit covers recovery); v2 backups/archives work with or
    without a local recovery key.
  - Codebase modularization: split `apps/desktop/src/library.rs` into
    `apps/desktop/src/library/` (9 domain files), `apps/desktop/tests/ui.rs`
    into `apps/desktop/tests/ui/` (8 test modules), and
    `apps/desktop/src/views/account.rs` into `apps/desktop/src/views/account/`
    (5 files).
  - Desktop security hardening: `0o600` atomic writes for Emergency Kit & vault
    archives, `VaultSession` stuck-unlocking recovery, `end_account_session`
    sync cleanup, `PasswordsView.recovery` cleared on lock, `envelope::seal`
    zeroizing buffer.
- **Passwords under the Ottero account (ADR 0014, done):**
  signed in, Passwords opens with the account. An existing vault asks for
  its master password once ("Use your Ottero account", or Settings →
  Privacy & data → Unlock with → Use Ottero account…, or the palette); the
  vault key is re-wrapped under `HKDF(account_key)` in one transaction and the
  master password is retired (recovery key required, made in the same write
  if missing). New vaults while signed in need no master password. One
  account unlock opens both; locking either locks both; sign-out locks
  Passwords and keeps the link; another account can't open it (clear screen,
  recovery key offered); recovery moves it to the open account or back to a
  master password. Backups of linked vaults are archive v2. Migration 0012
  rebuilds `vault_meta` with CHECKs so no vault can lose its last opener.
  Self-review: `docs/reviews/VAULT-LINK-SELF-REVIEW.md`.
- **Ottero Cloud accounts in the app (plan 06 step 3, done, not pushed):**
  Settings → Account is the first page: intro, Create account (strength
  meter, confirm), 6-digit code (paste-friendly, resend after 30 s, change
  email), Emergency Kit (Secret Key + recovery code, copy, printable HTML kit,
  required checkbox before keys upload), Sign in (asks for the Secret Key when
  this device has none), signed-in page (avatar, plan/Owner, sync "Coming
  next", Secret Key behind the password, kit, devices with sign-out). Actions
  + palette: Open account settings, Create account, Sign in, Sign out…, Copy
  Secret Key, Save Emergency Kit…. Crypto in `ottero_crypto::account`
  (ADR 0011, vectors cross-checked with Node/noble). Local state in
  `account.json` (0600); account key only in memory (Unlock after restart).
  Verified end to end against the local API (`ottero-cloud/api/dev.sh`) with
  two data folders: sign-up on A, Secret Key sign-in on B, both devices
  listed. See `docs/security.md` → Ottero Cloud account.
- **Two-step verification (TOTP) in the app (done, not pushed):** sign-in
  asks for the authenticator code (or a backup code) after the password;
  Settings → Account → Security → Two-step verification shows On/Off with
  Turn on… (password → QR code + setup key → code → 10 backup codes behind a
  required checkbox), New backup codes… and Turn off… (confirm, password).
  Palette: Turn on/off two-step verification…, New backup codes…. The
  replacement session token is saved; late answers are dropped and their
  sessions signed out. QR via `qrcode` (ADR 0013). `scripts/check.sh` passes
  (unit 156 core + 79 desktop lib, 92 UI). Verified live against the API.
- Rename to Ottero is done in code, packaging and docs (ADR 0009): crates
  `ottero-*`, binary `ottero`, `Ottero.app` (exe `Ottero`, helper
  `OtteroUpdater.app`), `Ottero.exe`, env vars `OTTERO_*` (`CALMFOLD_DATA_DIR`
  still read as a fallback), themes "Ottero Light/Dark", new Quiet Loop icon.
  First launch moves the Calmfold (or Keepstone) data folder to
  `com.Nexdark.Ottero` / `Nexdark\Ottero\data` and renames the database to
  `ottero.sqlite3`. Kept for compatibility: bundle ID `com.nexdark.calmfold`,
  Velopack packId `Calmfold`, vault AADs/archive format, `calmfold-releases`
  feed URLs, signing keys. Vault backups now `.otvault`; `.cfvault`/`.ksvault` open.
- Done: notes editor, tasks, Today, calendar (exceptions, ends, ICS import,
  deleted events, holidays for ~245 countries + states, birthdays), habits,
  focus/pomodoro timer (task-linked, header pill when active), password vault
  (recovery, backups), app icon, release workflow (`release.yml`, unsigned
  Windows + macOS zips on `v*` tags).
- Developer workspace: optional sidebar destination with searchable projects,
  links/folder references, real setup Notes and project Tasks, archive/restore,
  and encrypted environment sets shared with Passwords. Migration 0011 and
  item-data v2 for environments only; crypto envelope/KDF unchanged. See
  `docs/plans/11-developer.md` and `docs/security.md` for limits and export policy.
- Developer/environment audit follow-up and Workspace customization are fixed
  and verified; the latest quality gate passes 345 tests (75 UI tests).
  Settings → Workspace controls sidebar order/visibility with direct arrows,
  switches and reset/Undo. Hidden features live in inline More; keyboard and
  command access remain available. General selects the startup page.
- Website moved to the private repo `MohammedSafwan10/calmfold-site`
  (with the brand explorations formerly in `output/`). The app's changelog
  is `apps/desktop/changelog.json` (embedded in What's new, used by
  `release-metadata.py`); the site reads it from this public repo.
- Production updates: native Sparkle/Velopack replace the legacy installer.
  Final GPL candidate run **43918679795**, source **7d9b04f**, passes every job:
  Mac 346 tests, Windows 345, real installed upgrades/relaunch and tamper
  rejection, native packaging and production-key signature verification.
  Full local gate passes; changed Settings/What's new UI was reviewed in both
  themes. See `docs/plans/13-production-updates.md` and ADR 0008 for limits.
- Public project guidance: refreshed README with real sample-data screenshot,
  GPL-3.0-only LICENSE/NOTICE, package dependency notices, CONTRIBUTING,
  SECURITY, CODE_OF_CONDUCT and concise PR template. Private vulnerability
  reporting enabled. Shared `.kiro/skills` remains tracked; local state ignored.

## Next steps (in order)

**Passwords under the account (2026-11-18):** push the commits; owner review
of the new Passwords screens by clicking through once (real-window input was
impossible here: the display was off, see Log). Open items from the
self-review: account-key rotation (plan 06 step 8) must re-wrap the vault.
*(Idle and sleep auto-lock for an open account key without a vault was fixed
and tested on 2026-11-24.)*

**Next steps (desktop app):**

1. Automatic background sync scheduler + WebSocket live push listener in
   `apps/desktop/src/sync/`.
2. Multi-device account-wrapped `vault_key` sync record (`vault_meta`) so
   synced Passwords decrypt cleanly on a second device.
3. Release builds: store session token + Secret Key in the OS keychain once
   Developer ID signing exists (`docs/security.md`); add recovery-code
   password reset and QR device approval.

**Ottero rename follow-ups (before releasing 0.1.3):**

1. Push and let `update-probes.yml` run on Windows: it now upgrades a
   `Calmfold.exe` Velopack install to `Ottero.exe` (same packId). Not run yet.
2. Audit and rename the GitHub repos (`calmfold`, `calmfold-releases`,
   `calmfold-site`) and the website domain (`calmfold.nexdark.com`, still
   shown in Settings → About), then move the feed URLs in a later release while
   the old URLs keep redirecting.
3. Release 0.1.3 and prove a real installed v0.1.2 Calmfold → Ottero update on
   both OSes. Existing Mac installs keep the folder name `Calmfold.app`.

**v0.1.2 is published and immutable:**
https://github.com/MohammedSafwan10/calmfold-releases/releases/tag/v0.1.2

Final build source: 7d9b04f672bd66bcfab85d88c5523c5b44d3ea8e; successful run
43918679795. All 11 uploaded assets match verified local SHA-256/size values.
Public feed/signature/appcast/checksum bytes match; pinned feed signature passes.
All five large package/source URLs are publicly reachable with correct lengths
(HEAD checks, not complete public re-downloads). Exact application source TAR is
attached; GitHub's automatic source archive in the downloads-only repo is metadata.
The old v0.1.0/v0.1.1 source-repo release entries are removed as requested;
tags/history retained. The downloads repository README now explains installation,
source archives, license and signing boundaries. No further publication step is
needed for this version. Never clobber immutable assets; use a new version.

Both pinned signing keys are proven by final CI and local verification. The
protected Sparkle seed lives outside the repo at
~/.config/calmfold/release-keys/sparkle.key; never print or commit it. No system
keychain or publisher signing identity was used. Release packages include GPL,
third-party notices and exact source references. Actual packaged Mac launch and
fresh isolated database creation pass; real user data was untouched.

Install the official release once manually for native updates. Debug data remains
separate; do not automatically migrate it. Windows portable installed upgrade is
proven; Setup.exe per-user first install is not. Apple notarization/Windows
publisher signing, remaining failure scenarios, metadata expiry/key rotation and
automatic database rollback are not claimed complete. See plan 13.

Everything in `docs/TASKS.md` is done except the 500 KB+ editor item in §1.
Developer's first release and Workspace customization are implemented. General
utilities, environment comparison, CLI injection and sync remain deferred.
What's new has native version navigation, narrow-window dropdown, offline grouped
notes and back/Escape; no media renderer yet (plan 14). No Coolify needed for
bundled notes or GitHub-hosted updates. Website is still a scaffold: finish its
pages/release metadata before deployment.

## Known issues / risks

- GPUI comes from `gpui-pre` snapshots via gpui-kit; upgrades can break APIs.
  Upgrade gpui-kit deliberately, rerun `scripts/check.sh`, look at every screen.
- macOS blocks `osascript` keystrokes here, so real-window input automation
  isn't possible; UI flows are covered by headless tests in `apps/desktop/tests/ui.rs`.
- If the Mac display is idle, windows are off-screen; `scripts/screenshot.sh`
  falls back to off-screen windows.
- Note list isn't virtualized yet (fine for hundreds of notes).
- Sidebar element id includes its width so drag-resize tracks the pointer
  without easing lag; side effect: submenu expansion resets after a resize
  (current route's submenu reopens). Fix properly by keying only while dragging.
- "System" reduce motion is read once at startup.
- Storage limits (VACUUM + FTS rowid, ASCII-only `lower()`): see `crates/storage/AGENTS.md`.
- Global quick-capture hotkey and tray icon are unproven with GPUI; prototype first.
- Vault: Touch ID only in signed release builds
  (not built); lock screen without sleep isn't detected. Backups open with
  the master password at export time only (no recovery key in the file).
- Calendar: one occurrence of a repeating event can be moved (drag, or the
  detail panel → "Only this event") or deleted on its own; changing the
  series' schedule resets those. A series can end ("Ends" in the panel).
  Not yet: "this and following", per-occurrence titles/notes. Deleted events
  live in Calendar → View options → Deleted events… (restore, delete
  forever, empty). Times are local wall-clock; `.ics` import converts UTC and
  `TZID` times to local on the way in (Windows zone names fall back to
  local). No ICS export or live subscriptions yet.
- Calendar month cells fit as many lines as the window height allows (estimated
  from the viewport, `MONTH_CHROME`); the rest go behind "N more".

## Useful commands

```bash
rm -rf /tmp/ks && cargo run -p ottero-storage --example seed -- /tmp/ks/ottero.sqlite3
OTTERO_DATA_DIR=/tmp/ks OTTERO_DEV_ROUTE=notes cargo run   # debug: open on a screen
cargo run -p ottero-desktop --example seed_developer -- /tmp/ottero-review/ottero.sqlite3
scripts/screenshot.sh target/shot.png
OTTERO_DEV_ROUTE=notes OTTERO_DEV_SOURCE=1 cargo run    # debug: first note as Markdown source
OTTERO_DEV_ROUTE=calendar OTTERO_DEV_CALENDAR=week cargo run  # debug: calendar view (month|week|day|agenda)
```

## Log

### 2026-11-24: Events & Habits sync payloads + sign-out sync cleanup

- Extended `apps/desktop/src/sync/record.rs` with `SyncEventPayload` and
  `SyncHabitPayload` (reusing `Recurrence`, `Occasion`, `Exception`, and
  `Schedule` canonical codes) plus `apply_event` / `apply_habit`.
- Wired `Events` and `Habits` mutations in `Library` into `sync_outbox`,
  emitted `EventsChanged`/`HabitsChanged` in `run_sync_once`, and cleared local
  sync state (`clear_sync_state`) on `account_sign_out` (tested in `ui.rs`).
- Verified with `./scripts/check.sh`.

### 2026-11-24: Library outbox hooks & `run_sync_once`

- Wired `Library::record_sync` into Note, Task, Project, and Focus write/delete
  methods in `apps/desktop/src/library.rs`: enqueues to `sync_outbox` with a
  persistent per-install `device_id` only when signed in (local-only users stay
  at 0 outbox rows).
- Added `Library::pending_sync_count`, `Library::sync_state`, and
  `Library::run_sync_once` (runs `sync_once`, applies decrypted remote changes,
  and emits `NotesChanged` / `TasksChanged` / `FocusChanged`).
- Added UI test `signed_in_library_mutations_enqueue_sync_outbox_while_local_only_does_not`
  in `apps/desktop/tests/ui.rs`. Verified with `./scripts/check.sh`.

### 2026-11-24: Sync record serialization & per-field LWW remote apply

- Added `apps/desktop/src/sync/record.rs`: wire payloads (`SyncNotePayload`,
  `SyncTaskPayload` with `changed_fields`, `SyncProjectPayload`,
  `FocusSession`), `enqueue_local_change` (ticks HLC + writes `sync_outbox` +
  records local field clocks), and `apply_decrypted_change` (true per-field LWW
  merge for Tasks so concurrent edits to different fields converge without
  clobbering).
- Added two-device convergence & concurrent per-field LWW conflict merge test in
  `apps/desktop/src/sync/mod.rs`. Verified with `./scripts/check.sh`.

### 2026-11-24: Desktop sync client & encrypted push/pull engine (`apps/desktop/src/sync/`)

- Added `apps/desktop/src/sync/client.rs` (`SyncClient` over `ureq` HTTPS-only
  except loopback, mapping `401`/`402`/`410 CursorExpired`/typed `SyncErrorCode`).
- Added `apps/desktop/src/sync/engine.rs` (`ensure_collection_keys`,
  `build_push_batch`, `sync_once`): seals outbox changes per collection key +
  `item_aad`, drains batches, pulls and verifies remote HLCs/blobs, skips own
  device echoes, and sets `needs_full_resync` on HTTP 410.
- Added in-process fake sync server test proving two-device E2EE push/pull,
  ciphertext-only server storage, tamper rejection, and `CursorExpired` handling.
- Verified with `./scripts/check.sh`.

### 2026-11-24: Per-collection sync crypto + migration 0013 (`sync_state`, `sync_outbox`)

- `ottero_crypto::sync`: HKDF-SHA256 collection-key wrapping (`wrap_collection_key`
  / `unwrap_collection_key`, bound per collection code in AAD) and item payload
  sealing (`seal_sync_item` / `open_sync_item`, zeroizing plaintext).
- Migration `0013_sync.sql` + `ottero_storage::sync`: `sync_state` singleton
  cursor, `sync_collection_keys`, `sync_outbox` (HLC-ordered batch drain + ack),
  and `sync_field_clocks` (strict per-field LWW `set_field_clock_if_newer`).
- Verified with `./scripts/check.sh` (new crypto + storage unit tests pass).

### 2026-11-24: `crates/sync-protocol` (Plan 06, step 5 foundation)

- Added IO-free shared crate `crates/sync-protocol` (`ottero-sync-protocol`)
  for both the desktop client and `sync.nexdark.com`:
  - `hlc`: `Hlc` + `HlcClock` with 60 s forward-drift guard and fixed-width
    canonical string formatting whose lexicographical order matches `Ord`.
  - `collection`: 8 syncable collections (`notes`, `tasks`, `events`, `habits`,
    `focus`, `projects`, `settings`, `vault`) and separator-safe `item_aad`.
  - `wire`: versioned `PushRequest`/`PushResponse`, `PullRequest`/`PullResponse`,
    idempotent `ChangeId`, 1 MiB blob / 200-change / 8 MiB batch limits,
    `QuotaUsage`, `SyncErrorCode` (incl. `CursorExpired` for 90-day tombstone
    GC), and WebSocket notices (`SeqAdvanced`, `DeviceRevoked`,
    `EntitlementChanged`, `Ping`/`Pong`).
- 8 unit tests; `scripts/check.sh` passes.

### 2026-11-24: Account key idle & sleep auto-lock

- `Library::account_key_opened` now always calls `self.touch_vault()` so
  unlocking an account resets the idle clock even when Passwords is not set up
  or stays locked.
- `PasswordsView::check_auto_lock` now runs whenever `vault_state() == Unlocked`
  **or** `library.account_key_open()`, so both idle timeout and sleep lock an
  open account key without a vault.
- Added UI regression test
  `account_key_auto_locks_after_idle_and_sleep_without_a_vault` in
  `apps/desktop/tests/ui.rs`. `scripts/check.sh` passed.

### 2026-11-18: Passwords unlock with the Ottero account (ADR 0014)

- Crypto `ottero_crypto::vault_link`: HKDF(account_key, "ottero vault v1")
  + vault envelope, AAD "ottero vault key account v1"; fixed vectors made
  with Python hmac/hashlib and @noble/ciphers; tamper/AAD/length tests.
- Core: `next_opener` (MasterPassword | Account; SetUp, Link, Recover,
  Reset) with exhaustive table test.
- Storage: migration 0012 (rebuild `vault_meta`, nullable master columns,
  `account_email`/`account_wrapped`, CHECKs); `link_vault_to_account` (one
  transaction, compare-and-swap); tests for upgrade, CHECKs, and an
  interrupted link (abort trigger on a file DB).
- Desktop: `VaultSession` link/unlock-with-account/recover-to-account,
  archive v2; `Library::{link_vault_to_account, set_up_vault_with_account,
  account_key_opened, account_key_closed}`; `lock_vault` also locks the
  account key; sign-out and ended sessions lock Passwords. Passwords screens:
  Link, Unlock Ottero, Sign in to open, Another account, Can't open, Set up
  with account; recovery dialog relinks to the open account; restore of v2
  backups with the account or recovery key; `UseAccountForPasswords` action;
  Settings row "Unlock with". Dialog Enter no longer closes the change/forgot/
  archive/link dialogs (`on_ok` → false). Debug hook `OTTERO_DEV_ACCOUNT`.
- Tests: 11 new UI flows (local-only unchanged, link wrong→right, link dialog,
  new vault under account + restart → one unlock, auto-lock locks both,
  sign out/in, other account + recovery key, recovery while signed out, late
  link after account switch, late account unlock, linked backup restore),
  10 vault unit tests, 5 crypto, 2 core, 5 storage.
- Live: local API on :8787 (`ottero-cloud/api/dev.sh`), data folder
  `/tmp/ottero-vault-link` with an 8-item master-password vault: sign-up,
  wrong then right master password (link), restart, wrong then right account
  password (one unlock, 8 items), sign out (locked, 0 items in memory), sign
  in (reopened), lock. Driven through `Library` in a temporary headless test
  against the real HTTP API, because the Mac display was off: no window was
  on screen, so `click.swift` and typed input didn't reach the app. Screens
  were looked at as rendered by real launches on prepared data folders
  (Link, Unlock Ottero, Sign in to open, Another account, Set up with
  account) in light and dark; copy shortened after looking. Not looked at in
  a real window: the link dialog, the recovery dialog's relink text and the
  Settings "Unlock with" row (covered by UI tests only).

### 2026-11-17: Two-step review fixes (ottero-cloud docs/reviews/TWO-STEP-REVIEW-2026-11-17.md)

- API (ottero-cloud 35d2498, deployed): confirming two-step setup ends every
  other session of the user, so a password-only session can never become a
  staff session; a normal two-step sign-in elsewhere ends nothing. Regression
  covers enable, disable and re-enable, and fails on the previous code.
- App: `private_file::write_private` writes secrets to a new 0600 temp file and
  renames it over the target, so an existing 0644 Emergency Kit can't keep its
  permissions; account.json uses it too. Tests cover new files, overwriting a
  0644 file, no leftover temp files and failed writes. Windows ACLs of a saved
  kit are not checked yet (it inherits the chosen folder's ACL).

### 2026-11-17: Two-step verification (TOTP) in the app

- Core: `AccountState::TwoStep` + `TwoStepRequired` (table test updated),
  code/backup-code/setup-key helpers.
- Client: `sign_in` → `SignInOutcome::{Session, TwoStep(challenge)}`; the
  `better-auth.two_factor` cookie stays in memory and is sent with
  `Origin` = server origin (found live: Better Auth refuses cookie requests
  without a trusted origin, `MISSING_OR_NULL_ORIGIN`). enable / confirm
  (replacement token) / disable (also replaces the session, found in the
  plugin source) / generate-backup-codes; error mapping for wrong code,
  wrong/used backup code, lockout (429 `ACCOUNT_TEMPORARILY_LOCKED`), expired
  challenge, too many tries, changed elsewhere.
- Fake: RFC 6238 via `ottero_core::vault::totp`, single-use backup codes,
  5-failure lockout, per-challenge attempt budget, session replacement.
- UI: TwoStep sign-in screen (auto-submit on 6 digits, paste "123 456",
  backup-code toggle, Back/Escape), Security row, `TwoStepDialog`
  (locked while busy and on the backup codes), actions in the palette.
- Tests: client (in-process HTTP), fake, 6 new UI tests incl. two late-result
  tests and "unlock needs only the password; sign-in needs the code".
  `account_other_device_can_be_signed_out_after_confirming` now scrolls to
  Devices (the new row pushed it below the test window).
- Live: separate API on :8788 (DB `accounts_2fa`, mail
  `/tmp/ottero-mail-2fa.jsonl`) because an older dev server was already on
  :8787. Sign up, turn on (wrong + right code), sign out, sign in with wrong
  then right code, backup code, turn off, turn on again, in dark and light.

### 2026-11-17: Account review fixes (ottero-cloud docs/reviews/ACCOUNT-REVIEW-2026-11-17.md)

- API (ottero-cloud d08c056, deployed): public Better Auth surface is an
  allowlist (sign-up, sign-in, verify/resend email code, sign-out, session
  list/revoke, token, jwks). Email-code password reset/sign-in, password and
  email change, update/delete user and every /admin/* plugin route now 404;
  verification-code requests for any type but email-verification are refused.
  Only the owner acts on staff; role changes go through an audited route.
  Tests sweep every Better Auth endpoint and fail on the previous code.
- App: refresh and unlock results are bound to the session epoch and token;
  sign-in, sign-out, session end and lock start a new epoch. Three UI tests
  (late refresh, late unlock after an account switch, late unlock after
  sign-out) failed before the fix and pass now.

### 2026-11-17: Ottero Cloud sign-up, sign-in and account page

- `crates/crypto/src/account.rs`: Secret Key (O1 + Crockford), recovery code
  (checksummed), Argon2id ⊕ HKDF(Secret Key) → auth/wrap keys, account key
  wraps; 17 tests incl. RFC 5869, fixed vectors (test + real params), tamper.
  New deps `hkdf`, `unicode-normalization`, `base64` (ADR 0011).
- `crates/core/src/account.rs`: email/password rules, `AccountState`
  machine with exhaustive transition table, plan labels, device labels.
- `apps/desktop/src/account/`: ureq client (timeouts, typed errors, https-only
  in release, tests against an in-process HTTP server), `account.json` store,
  flows on the background executor, `FakeAccountApi`, kit HTML.
  `ottero_desktop::start_with_account_api` lets UI tests inject a server;
  `start` uses `OTTERO_ACCOUNT_URL`. UI tests never touch the network.
- `views/account.rs` + Settings page 0. Settings page indexes moved by one.
- Verified: check.sh; real two-instance run (screens in dark and light);
  wrong Secret Key typo and wrong key messages; copy buttons proven by UI test
  (real clicks via `click.swift` were unreliable on small icon buttons).
- Not verified: Windows; the kit's print layout in a browser this session;
  production server (never used).

### 2026-11-17: Ottero 0.1.3 released; site live; faster CI

- Published immutable v0.1.3 in calmfold-releases (11 assets: Mac zip, Windows
  Setup/Portable, Velopack nupkg, signed stable.json + minisig, Sparkle appcast,
  exact source tar from 54c3e26, LICENSE, NOTICE, SHA256SUMS). Verified locally:
  minisign signature with RELEASE_KEY, sha256/size of both platform packages,
  Mac bundle name/ID/feed, ran the downloaded Mac build. Windows CI proved the
  real Calmfold.exe -> Ottero.exe Velopack upgrade and tamper rejection.
- Repos renamed: calmfold -> ottero, calmfold-site -> ottero-site (GitHub
  redirects). calmfold-releases keeps its name: installed apps' feed URLs.
- Website live at https://ottero.nexdark.com (Coolify project "Ottero", app
  ih3sfzq4hni4epwarrdj8mog, read-only deploy key; GitHub webhook deploys on push;
  no Actions needed). In-app About and docs point to it.
- CI: `profile.ci` (opt-level 0, no debug info; OTTERO_CHECK_PROFILE=ci in
  check.sh), Windows Dev Drive, and release.yml skips the repeat quality gate
  when CI already passed on the tagged commit. CI went from 15m/21m to 6m/10m.

### 2026-11-17: Save approved Ottero icon master

- Owner chose concept 01 (curled charcoal otter/O). Generated a clean square
  source image without presentation labels or wordmark; opaque ivory canvas so
  platform masks can be derived separately. Saved master, original reference and
  generation prompt in assets/brand/ottero with a rename handoff README.
- Visually inspected the source and verified the saved copy matches its generated
  file. No runtime assets replaced, repositories renamed or user data touched.

### 2026-11-17: Repo split

- `website/` and `output/` moved to private `calmfold-site`; changelog moved
  to `apps/desktop/changelog.json` (paths updated in CI, release metadata,
  What's new, testing doc). `cargo clean` + full gate: 346 tests, target 3.4 GB.

### 2026-11-16: Publish verified GPLv3 native release

- Publish complete v0.1.2 draft in calmfold-releases only after all 11 uploaded
  asset digests/sizes match local verified files. GitHub confirms latest and
  immutable; public metadata/signature checks and all package HEAD checks pass.
- Final source 7d9b04f; run 43918679795 passes Mac/Windows quality, real native
  installed upgrades/tamper rejection, packaging and signing. Local full gate
  passes 346 tests; final Mac package launches with a fresh isolated database.
- Attach exact application source TAR, LICENSE/NOTICE and SHA256SUMS; update
  downloads README. Old source release entries v0.1.0/v0.1.1 removed only after
  public verification; retain tags/history. No real user data changed.
- First official installation remains manual; future native in-app updates use
  Settings → About. Setup.exe first-install and publisher signing remain open,
  along with the explicitly unchecked production-hardening gates in plan 13.


### 2026-11-16: Repair cached native SDK packaging and Windows text decoding

- GPL candidate run 43915646801 passes all app/native tests, but packaging caught
  an incomplete Sparkle framework restored from the Rust cache and Windows
  cp1252 decoding of Cargo's UTF-8 JSON. Nothing was published.
- Put CI SDKs in ignored .native-tools outside Cargo target caches. Scope the
  release Rust cache to target/release. Validate cached framework signatures;
  rebuild incomplete cached output/derived/source from checksum-pinned archives.
- Reproduced an incomplete framework locally in an isolated cache; builder
  repaired it, verified signatures and confirmed no WebKit linkage.
- Decode Cargo metadata explicitly as UTF-8 for dependency notice generation.
  Final publication still requires a fresh candidate containing these fixes.


### 2026-11-16: Adopt GPLv3 and refresh public project documentation

- Owner chose GPLv3 after discussion of private use, commercial redistribution
  and copyright enforcement. Set GPL-3.0-only, add canonical LICENSE and original
  copyright NOTICE; preserve third-party licenses and no trademark grant.
- Add license/dependency notices to Mac resources and Windows payload before
  sealing/packaging. This requires a fresh candidate before publication.
- Refresh README around the product, a real sample-data screenshot, downloads,
  supported tools and clear local-data boundaries. Add contribution/security/
  conduct guidance and concise PR template. Enable private vulnerability reports.


### 2026-11-16: Resume public release and prove Windows native updates

- Source repository visibility confirmed PUBLIC. Run 43872632747 starts with
  full app checks and both native engine jobs; both engine jobs pass.
- Windows 1.0.0 → 1.0.1 actual startup, sibling replacement, two-instance lock,
  unchanged external user data and catalog/package rejection verified.
- Owner requests old releases removed only after new v0.1.2 publication and
  download verification. Old source releases: v0.1.0 and v0.1.1. Keep tags/history.
- README now points to the canonical calmfold-releases repository and describes
  native packages and the complete draft-before-publication procedure.


### 2026-11-16: Keep shared Kiro guides, ignore local state

- `.kiro/skills` contains the checked-in, licensed GPUI reference guides required
  by AGENTS.md (about 432 KiB). Keep these shared instructions in Git; they are
  not application assets. Ignore other `.kiro` children by default for local
  machine/session state. No tracked files removed or history rewritten.
- Owner made the source repository public. Release candidate run 43872632747
  started successfully; final native Windows proof and publication still pending.


### 2026-11-16: Reduce repeated CI work without weakening release gates

- Restrict expensive app checks to app/build inputs, explicitly preserving the
  bundled release-history JSON. Native checks also include controller, signed
  catalog, packaging and dependency/toolchain inputs that were missing before.
- Cancel superseded branch/PR runs, avoid duplicate native feature-push + PR
  runs, retain useful caches after failures, bound runtime and artifact retention.
  Packaging now waits for both full quality and native engine gates.
- Cancel duplicate CI run 43869388968: its application source matched the
  successful Mac/Windows run 43868080626; only fixture/workflow files differed.
- Validate workflows with upstream actionlint 1.7.12. Full local gate remains
  required. New hosted execution was blocked at that checkpoint; owner subsequently made
  the repository public and candidate run 43872632747 started successfully.
- Current local Mac ZIP is approximately 17 MiB; build caches are not the
  installed/download size. No repository visibility or billing setting changed.


### 2026-11-16: Native offline release history

- Settings → About → Read release notes and command-palette What’s new open
  a full content-area reading view. Back/Escape returns to About; app navigation
  remains present. Bundled JSON provides version/date, installed marker, concise
  introductions and New/Improved/Fixed sections, shared with signed feed notes.
- Version rail shows one release at a time; a native dropdown replaces it on
  narrow windows. Tests cover old/current selection and dropdown navigation.
  Settings gear has no persistent filled tile; hover/focus remain native.
- Native flow passes opening, back and Escape in both themes. No WebView, remote
  HTML, startup popup or new permanent sidebar entry. Plan 14 tracks review.


### 2026-11-16: Adopt native update engines (release work in progress)

- Replace the custom installer and obsolete unbundled probes with Sparkle 2.10.1
  and Velopack 1.2.161. Shared signed metadata, bounded authenticated downloads,
  OS instance locks, retry backoff, save guards and SQLite recovery snapshots.
- Mac real installed fixture upgrades and relaunches; modified archives are
  rejected while the original stays running. Production nested-helper layout
  passes too. Build Sparkle without UI bits; verify no WebKit dependency.
- Add equivalent Windows installed-package, relaunch, sibling replacement,
  instance lock and catalog/package rejection checks in native CI. Windows
  results and release publication remain pending; do not claim completion.


### 2026-11-16: Start production updater plan and harden existing boundaries

- Added plan 13 and ADR 0008, with explicit installed-app/native-OS gates and
  staged publishing/data-recovery requirements. Keep experimental engine deps
  isolated from the app. Reviewed primary docs and actual crate APIs.
- Stream downloads with a 32 KiB buffer into private temporary files. Enforce
  signed size before writes and hash before unpacking. Reject unsafe release
  paths, malformed hashes, zero/oversized payloads and credential-bearing URLs.
- Atomically sync/persist ready state and propagate failure instead of reporting
  Ready. Hold the interprocess lock from feed fetch through staging. Preserve
  automatic-update preference changes made during a network request.
- Refuse update restart for failed Notes/Developer/Passwords saves, pending
  environment imports and open task sheets/dialogs. Preserve drafts/editors.
- Full gate: 345 tests including 75 UI tests. Native Settings shell reviewed
  light/dark; window was off-screen for real input, so flows use UI tests.
- Isolated Mac engine probes pass development guards; dependency tree has no
  direct GPUI/webview. Added read-only/no-secret Mac/Windows probe workflow.
  This is not a signature/download/install/relaunch test or a production engine.

### 2026-11-16: Finish Workspace customization and organize pending work

- Persist sidebar order, visibility and startup route in layout.json, independent
  of appearance reset. Preserve old installations; fresh installs start with
  Today, Notes, Tasks and Calendar. Unknown/new features remain reachable in More.
- Replace confusing drag controls with direct keyboard-accessible arrows and
  switches. Place More after visible features only when something is hidden;
  leave Settings alone in the footer. Reset supports Undo without changing startup.
- Organize Settings into General, Workspace, Appearance, Features, Privacy & data,
  Keyboard and About. Search reaches hidden-feature preferences. Customize sidebar
  opens Workspace directly. Stable focus survives reordered native settings rows.
- Quality gate passes 337 tests including 74 UI flows: migration/persistence,
  order/hide/all-hidden, startup, reset/Undo, appearance independence, draft/vault
  preservation and running timers. Reviewed window-only light/dark screenshots;
  closed review copies and reopened one real-data app. Windows was not run.
- Review pending work for separate commits. Credential/private-key pattern scan
  found no matches; this is not a formal security audit. Patch website Astro to
  7.4.12 (npm audit: zero known vulnerabilities). Website build remains blocked
  by unpublished release metadata and missing pages; document that explicitly.

### 2026-11-16: Resolve Developer/environment audit findings

- Verified the external review against current code. Preserve failed-validation
  and unapplied-import drafts when changing Passwords items, filters, tags or
  starting another item; the shared pane also refuses to replace a failed draft.
  Correcting the error allows normal save-and-switch behavior.
- Reveal now renders literal values (including multiline content); only the
  masked editing representation is escaped. Single/all-value copies update
  Recently used. Keys trim accidental edge whitespace; values are untouched.
- New Project forms exit on selection, archived-view changes, New environment
  and Escape. The locked Secrets view owns unlock guidance, so no stale banner
  survives unlock. Preserve genuine validation errors instead of clearing all
  errors on every data event.
- Export snapshots while unlocked after confirmation/location selection, then
  writes/syncs in the background. It reports safe specific errors, prevents
  simultaneous exports and still refuses overwrites with private Unix files.
  Lock before the path picker returns prevents the export; an already-started
  explicitly approved export completes its snapshot.
- Remove the extra Developer Secrets scroll container, constrain the pane to
  its available height, size variable lists to their content (six-row cap), and
  expand revealed multiline rows within bounds. Measure actual escaped UTF-8
  output size instead of rejecting ordinary files above approximately 512 KiB.
- Archive-view menu is a real checkable app action (including unchecked native
  accessibility state), settings have Developer search keywords, and project
  deletion emits DeveloperChanged. Sidebar hiding intentionally preserves
  command/shortcut access; it is a visibility setting, not authorization.
- `scripts/check.sh` passes: 329 tests, including 69 native UI integration
  tests. New regressions cover failed navigation, pending imports, key trimming,
  literal multiline reveal, compact one-variable layout, copy usage tracking,
  export collisions/locked pickers, form navigation and archive menu state.

### 2026-11-16: Add Developer projects and encrypted environment sets

- Reuse task ProjectIds and real setup Notes; no second project or vault
  system. Projects can be created or promoted from Tasks, edited, archived
  with Undo, and restored. Hiding Developer in General settings keeps data.
- Add a shared masked variable editor to Developer and Passwords: searchable
  virtual rows, file/paste import with key-only preview, individual/all copy,
  deliberate plaintext export with a location prompt and no overwrite.
  Async imports cannot repopulate a locked or replaced draft.
- Domain parser accepts literal assignments, comments, quotes, multiline
  values and optional export prefixes. Rejects duplicates and oversized
  input with safe errors. Limits: 1 MiB, 512 variables, 64 KiB/value.
- UI testing caught the kit stripping newlines in masked inputs. Values now
  use a documented reversible escaped representation; pasted values escape
  automatically, and copy/export decode them. An 81-variable UI regression
  preserves multiline values through imports, edits, saves and export.
- Ordinary items remain item format v1; environment records use v2 and are
  tested through encryption/reload and encrypted backup recovery. Upgrade
  older clients before using environment items. No crypto primitive changes.
- Owner's visual feedback: replace centered spaced-out project buttons with
  compact left-aligned icon rows, clearer selected state, a narrower list,
  compact tabs and grouped resources. Reviewed native light/dark layouts.
- `scripts/check.sh` passes (324 tests, including 65 UI tests). Tests cover
  v10 upgrades, file persistence/corruption, project navigation/edit/archive,
  literal limits, backup compatibility, export confirmation/0600/no overwrite,
  clipboard clearing, lock cleanup and stale-dialog results. Real clicking
  was unavailable while the Mac's windows were off-screen; flows use real
  native components in headless UI tests.
- `seed_developer` creates a disposable visual-review fixture and refuses
  existing databases. Screenshot/click scripts accept `CALMFOLD_WINDOW_PID`
  so review windows can be targeted without capturing another Calmfold app.

### 2026-11-16: Simplify locked mode control after visual feedback

- Removed the always-visible helper sentence and the native per-tab disabled
  styling that created three separate dark blocks. Keep one standard
  segmented strip, subtly dimmed while active, with the explanation on hover
  only. Domain guards still reject mode changes without resetting the timer.
- The repeated-click regression verifies no toasts, no persistent helper text
  and preserved running/paused state in both themes. `scripts/check.sh` passes
  (309 tests). The owner's running timer was not restarted.

### 2026-11-16: Stop notification stacking on locked focus modes

- Removed the per-click mode-lock toast. Other modes are now disabled while
  running or paused, with a single quiet inline hint; the current mode stays
  selected. Mode choices re-enable and the hint disappears after Finish/Reset.
  Native TabBar uses `FocusKind::ALL` and its labels as the source of truth.
- Extended the real tab-click regression to repeat mode clicks 24 times per
  theme and verify zero notifications, preserved state and hint removal.
  `scripts/check.sh` passes (309 tests). Real-input automation remains
  unreliable here; the changed interaction is covered by the UI test in both
  light and dark. The owner's running session was deliberately left intact.

### 2026-11-16: Quieter focus controls and searchable task selection

- Transport controls (including Start) are icon-only, transparent at rest,
  retaining native hover/focus treatment and shortcut tooltips. Duration is
  one compact menu above the task link and controls, with mode-specific
  presets and inline custom minutes; invalid custom values leave the timer
  unchanged and show the permitted range from core constants.
- Replaced the first-15-task popup with the existing GPUI Kit searchable
  single-select Combobox, a bounded scrollable menu, stable TaskId values,
  task-title truncation, keyboard navigation and explicit empty/error states.
  Selection follows linking, unlinking and task reloads without writes on
  programmatic synchronization. Tasks are filtered in memory; no search IO.
- Added UI tests searching 301 tasks and selecting outside the previous
  cutoff, unlinking, duration presets and invalid/valid custom input.
- `scripts/check.sh` passes: 309 tests including 62 UI tests. Ran the app
  against throwaway data and inspected light/dark Focus screenshots. Real
  scripted clicks still did not activate controls; keyboard and picker flows
  were verified with headless UI tests. Windows was not run on-device.

### 2026-11-16: Protect active focus timers from mode-tab resets

- Mode selection now changes an idle timer only. Running and paused sessions
  retain their countdown, original start, cycle and task link; selecting the
  current tab is a no-op. Attempting another mode explains that Finish or
  Reset is needed. Starting focus from another task cannot overwrite an
  active block either.
- Added a core mode/state matrix regression test and real tab-click UI
  coverage in both themes, plus a real SQLite foreign-key save-failure/retry
  test proving that progress is retained and exactly one session is saved.
- Fixed Focus task-completion Undo to restore InProgress via the legal
  reopen transition; added a UI regression. Cleared existing lint errors in
  the custom-duration blur handler and updater (no updater behavior changes).
- `scripts/check.sh` passes (format, strict Clippy, all workspace tests).
  Headless UI coverage includes running/paused modes, Reset, task linking,
  navigation, save retry and completion Undo.
- Ran the app with throwaway data and visually checked Focus in light and
  dark using window-only screenshots. Closed the review instances; the
  owner's existing app process was left running. Windows was not tested.

### 2026-11-16: Focus / Pomodoro timer

- `core::focus` (`FocusTimer`, `FocusKind`, `FocusPreset`, `FocusSession`,
  `TaskFocusStats`): wall-clock drift-free state machine (`Idle`, `Running`,
  `Paused`) tracking `session_started_at` separately from `segment_started_at`
  so pauses never skew logged start times, plus Pomodoro cycle tracking and
  duration formatting.
- Migration `0010_focus.sql` + `storage::focus`: `focus_sessions` table
  (`ON DELETE SET NULL` on `task_id`), range queries, and per-task focus
  aggregations (Focus blocks only).
- `apps/desktop/src/focus.rs` (`FocusController`): app-wide entity spawning a
  1s tick only while `Running`, registered as a global so task rows and task
  details can start focus directly.
- `views/focus.rs`: calm `ProgressCircle` countdown, 4 cycle dots, linked task
  picker, duration presets (`15m`, `25m`, `50m`, `90m`), gentle completion
  banner (`Start break`, `Mark task done`), and Today's log with undo delete.
- `shell.rs`: quiet title-bar pill (`● 18:42 · Task`) shown only while active
  off the Focus screen, `GoFocus` (`⌘7`), `FocusToggle`, `FocusReset`,
  `FocusFinish`, `FOCUS_SHORTCUTS` in Settings → Keyboard, and midnight reload.
- `task_row.rs` & `task_detail.rs`: `"Start focus"` action and live-updating
  per-task focus totals (`LibraryEvent::FocusChanged`).
- Verified with `cargo fmt --check`, `cargo clippy` (0 warnings in Focus files),
  and `cargo test --workspace` (including `focus_timer` UI test in `ui.rs`).

### 2026-11-11: Build disk fixed

- `[profile.dev]` debug = line-tables-only, dependencies debug = false.
  Clean `cargo build` + `scripts/check.sh` (clippy + all tests): target
  4.0 GB (was ~60 GB). `scripts/clean-build.sh` (cargo-sweep, 7 days)
  keeps it level; run weekly. Local release/perf builds are optional (CI
  builds releases).
- Windows: the title-bar panel toggles `occlude` the drag area, so mouse
  clicks reach them (Windows hit-tests every hitbox and reported caption).
  Today's empty notes hint says Ctrl+N off macOS. Not yet checked on Windows.
- Website work in `website/` (Astro 7.4, real screens via
  `scripts/capture-screens.sh`) is in progress and not committed.

### 2026-11-11: Paused development; cleared generated builds and caches

- At the owner's request, ran `cargo clean` and removed Cargo's registry
  downloads, extracted sources, index and Git dependency caches, plus
  `website/node_modules`. Source, lockfiles, branding and app data were kept.
- Workspace now occupies about 145 MB; disk available space rose from
  about 167 GiB to 230 GiB. Cargo reported removing 87.5 GiB of logical build
  files (about 62 GB as measured by `du` before cleaning).
- On resuming, Cargo will download dependencies and rebuild; run the
  website's package-manager install before working on it. No build/tests
  were run after cleanup, to avoid recreating the removed artifacts.

### 2026-11-09: Renamed to Calmfold (by Nexdark Labs)

- Keepstone → Calmfold everywhere: crates (`calmfold-*`), binary `calmfold`,
  app ID `com.nexdark.calmfold`, menus, themes ("Calmfold Light/Dark"),
  env vars `CALMFOLD_*`, docs. About shows "Made by Nexdark Labs".
- Data: the old `app.Keepstone.Keepstone[-Dev]` folder moves to
  `com.Nexdark.Calmfold[-Dev]` on first launch and `keepstone.sqlite3` is
  renamed (`paths.rs`, tested). Saved theme names map over. Vault backups
  are now `.cfvault`; `.ksvault` still opens.
- Kept on purpose (format identifiers bound into existing data): vault AADs
  `keepstone-vault-key-v1`, `keepstone-vault-recovery-v1`, archive format
  `keepstone-vault-archive`.
- App icon: `assets/brand/source/icon-master.png` → `scripts/gen-icons.py`
  (sizes, `.icns`, `.ico`, favicons + webmanifest). `scripts/bundle-mac.sh`
  builds an unsigned Calmfold.app; Windows embeds the icon via `build.rs`
  (winresource, untested on Windows). About screen shows the icon.
- GitHub repo is still named `keepstone` (rename on GitHub if wanted).

### 2026-11-09: States, birthdays, holiday updates

- Holiday table now from python-holidays 0.108 (Nov 2 2026), generated with
  `uv run scripts/gen-holidays.py` (inline script metadata pins the version).
  Adds 629 states/regions (`S` lines, `IN-TN`); a state only lists days the
  country lacks or that are a day off only there (Pongal in Tamil Nadu).
  Settings → Calendar → Holidays → "States and regions" (searchable).
- Cells use `holidays::short_name` ("Pratihar Shashthi"); the day panel and
  agenda wrap full names; tooltips show everything.
- Birthdays/anniversaries: `core::Occasion` (kind + optional year), migration
  0009, quick add ("Mom's birthday nov 20 1966", "anniversary feb 14 since
  2019"), panel Type (Event/Birthday/Anniversary) + Born/Since year, "turns 60"
  / "5th anniversary" in titles, cake/gift icons, quieter fill, View options
  toggle (`calendar_occasions`). Imported yearly "… birthday" ICS events
  become birthdays.
- Upkeep: `.github/workflows/holidays.yml` opens a PR monthly with the latest
  package's data; a core test fails 3 years before the data ends.

### 2026-11-09: Holidays

- `core::holidays` reads a table generated by `scripts/gen-holidays.py` from
  python-holidays 0.83 (MIT): ~245 countries, 2025–2036, public holidays +
  observances, estimated lunar dates, zone→country map (ADR 0007).
- `apps/desktop/src/holidays.rs`: settings (appearance: `holidays`,
  `holiday_observances`, `holiday_countries`, `holidays_where_you_are`),
  first-launch country from the time zone, travel country added with a label,
  `DayHolidays` per visible range. Calendar reloads when the zone changes.
- Shown as quiet text: month cell header, week/day column header, day panel,
  agenda, Today header and "Next 7 days" day headings. Tooltip and panel say
  "Public holiday in India" / "Observance" / "Date may shift by a day".
- Filters: View options (holidays, observances, where you are, choose
  countries) and Settings → Calendar → Holidays (searchable multi-select).
- Checked in the real window (month view, light, India). Tests: 5 core, UI
  `holidays_show_quietly_follow_settings_and_travel`.
- Regenerate yearly: see the script header.

### 2026-11-09: Calendar import and series ends

- `core::ics::parse_ics` (pure; zone lookup passed in): unfolding, escapes,
  VALARM skipped, DATE/floating/UTC/TZID times, DURATION, RRULE
  (DAILY/WEEKLY+BYDAY/MONTHLY/YEARLY, INTERVAL, UNTIL, COUNT), EXDATE,
  RECURRENCE-ID overrides, CANCELLED. Unsupported rules import their first
  date and are reported. Limits: 20 MB, 20,000 events.
- `Event::until` (`set_until`, migration 0008); panel "Ends" picker
  (placeholder Never, clearable); summary "Every week until Dec 1, 2026".
- `Library::import_ics` skips events already present (title, times, rule).
  `ImportCalendar` action: File menu, ⌘K, Calendar → View options.
- Tests: 6 ICS + 1 until (core), storage round-trip, UI `ics_files_import_once`.

### 2026-11-09: Deleted events

- `views/event_trash.rs`: sheet listing deleted events (Restore, Delete
  forever, Empty trash…, both confirmed). Opened from Calendar → View
  options or ⌘K "Show deleted events" (`ShowDeletedEvents`). Library:
  `deleted_events`, `delete_event_forever` (trashed only), `empty_event_trash`.
  UI test `deleted_events_can_be_restored_or_removed_for_good`.

### 2026-11-09: Change one occurrence of a repeating event

- `core::event::Exception` (Skipped / Moved, keyed by series date) with
  `skip_occurrence`, `move_occurrence`, `restore_exception`,
  `occurrence_on`; `occurrences()` applies them (moved ones can land outside
  their week). Schedule changes clear exceptions; text changes keep them.
  Migration 0007 (`events.exceptions`, text code). `Occurrence` gained
  `series_date` and `moved`.
- Detail panel opens at the clicked occurrence; date/time changes and Delete
  ask "Only this event / All events" (`feedback::ask_repeat_scope`). Dragging
  a repeat moves only that one, with Undo. Tests: 5 core, 1 storage, UI
  `one_occurrence_of_a_repeating_event_can_change_on_its_own`.
- Not checked on screen (real clicks don't reach the window); the flow is
  covered by the UI test.

### 2026-11-09: Encrypted vault backup

- Export vault backup / Restore vault backup (actions, File menu, ⌘K,
  Settings → Passwords → Encrypted backup). Same `.ksvault` format as
  archives, written atomically and verified; restore skips items already in
  the vault. ⌘K also gained Change master password, Create new recovery key,
  Show archived vaults. UI test `encrypted_backup_exports_and_restores_without_duplicates`.

### 2026-11-09: Vault next level + recovery

- Custom fields (text, hidden, website, date, one-time code) on every item;
  templates (Wi-Fi, licence, API key, SSH key, identity, authenticator) under
  New → More, all on the three kinds (no format change).
- Search language (`core::vault::search`): ranked, AND words, never secrets,
  `tag: type: is:fav|weak|reused|old has:code site:`, typo fallback pass
  (letters only, only when nothing matches exactly).
- List: variable-height virtual list with "Recently used" (12+ items) and A–Z
  headers (30+, name sort); Sort by in the … menu; tag chip; sidebar shows the
  top 6 tags and "One-time codes" (live codes in the list).
- Recovery key + Emergency Kit, "Forgot master password?" (recover / archive
  and start over), Archived vaults (ADR 0006, migration 0006).

### 2026-11-09: Password vault

- ADR 0005 (key hierarchy, totp-rs, zxcvbn, EFF list, csv). `core::vault`
  (items, lock machine, generator, health, totp, import), migration 0005,
  `storage/vault.rs` (ciphertext only), `apps/desktop/src/vault.rs` (crypto
  glue, 7 unit tests incl. 2,000-item unlock), `vault_prefs.rs`, screens
  `passwords.rs`, `vault_item.rs`, `vault_generator.rs`, Settings → Passwords.
- External review (GPT) found 4 gaps, all fixed with tests: drafts survived
  locking, generator stayed open after lock, RNG failure fell back to 0, no
  sleep handling (now wall-clock idle + wake detection). Lock screen without
  sleep is still undetectable: documented in `docs/security.md`.
- Dev: `CALMFOLD_DEV_VAULT=<password>` (debug only) unlocks/creates a vault
  with samples; `CALMFOLD_DEV_VAULT_ITEM=<title>` selects one.

### 2026-11-09: Today rebuild

- Sections as data (`today_layout.rs`: key/title/width, resolve/move/rows,
  unit-tested); order + hidden keys in appearance settings. Customize menu on
  Today; Settings → Today reorders with up/down and switches.
- New: at-a-glance strip, schedule with now marker and "Now" badge, habits
  with checks, week card with "habits kept" row, merged next-7-days list.
- Checked in the real window (dark).

### 2026-11-09: Calmer calendar

- Owner feedback: month view noisy (Gym + Standup on every day). Routines now
  fold into dots, cells show titles first, a day panel lists the selected day
  in full (wide windows), View options can hide routines. Week blocks: routines
  quieter, short blocks one line, thinner now line. Checked month + week (dark).

### 2026-11-09: Habits

- Research: Streaks (one big tap per habit), Loop (strength score that
  forgives a missed day), Kadō's EMA spec (α = 0.05). Built our own EMA.
- `core::habit` (schedule, goal, tap, stats, quick add), `core::habit_insights`
  (expected-vs-done rates), migration 0004, `storage/habits.rs`, Library habit
  methods (`HabitsChanged`), screens `habits.rs`, `habit_insights_view.rs`,
  `habit_detail.rs`, `habit_widgets.rs`. Sidebar badge = habits left today.
- First design was a wall of check boxes; redesigned to dots + one check, in
  one card. Checked in the real window (dark): Today and Insights tabs.

### 2026-11-09: Calendar

- Research (Fantastical, Apple Calendar, Notion Calendar): one natural-language
  field, month/week/day/agenda, multi-day bars, side-by-side overlaps, a now
  line, drag to reschedule, ⌘←/⌘→ periods, T for today.
- Events are local civil times; all-day ends are exclusive midnights; monthly
  repeats count from the first date (31st stays at month end). Quick add reads
  only the end of the text; undated text lands on the selected day.
- Drag: month cells and the all-day row take events (timed → all-day) and tasks
  (sets the due date); the time grid snaps to 15 min and keeps the length.
  Double-click empty time or a day creates "New event" with its title selected.
- Calendar keys are bound only on the grid (`CalendarGrid` context), so typing
  in quick add never switches views (UI test covers it).
- Month view checked in the real window (dark). Week/day/agenda are covered by
  UI tests only: this session the reader couldn't open fresh screenshots and
  real keys/clicks didn't reach the window.

### 2026-11-09: Measured editor performance

- `scripts/perf.sh` + `--features perf` (`apps/desktop/src/perf.rs`): real HID
  keystrokes/scroll, GPUI foreground journal → latency, draw, late frames.
  `[profile.perf]` = release + symbols. `perf_seed` example makes 1.5 KB…2 MB notes.
- Vendored `zorite-editor` (`vendor/`, `[patch.crates-io]`) with 7 patches; 50 KB
  typing p95 341 → 23 ms with 0 missed refreshes. Details: `docs/perf.md`.
- Gotchas found: GPUI's `caused_invalidation` is false for keys (views notify
  after the handler); `set frontmost` via System Events leaves no key window;
  a full-screen app on another Space hides our window (no frames, no keys).

### 2026-11-08: Live-preview note editor

- Replaced the gpui-kit `Editor` + Preview with `zorite-editor` (audited: no
  `unsafe`, no production panics, env-gated logs). Same `gpui-pre 0.3.11` as
  gpui-kit; hard rule 2 now names this one exception.
- Host work in `note_editor.rs`: theme → `SyntaxStyle`, `[[` suggestions with
  capture-phase keys, selection bar (base `Button`, `focusable(false)`),
  block formats through `core::format` as single undo steps, math provider,
  `EditMath`/`EditProperties` handling (zorite stops the caret otherwise),
  tag-vs-link click routing, URL allow-list (http/https/mailto).
- Verified in the real window (dark + light): bold, links, highlights, colors,
  selection bar, Bold click saved `**Insurance**`.
- Removed: `TogglePreview` (⌘E is now inline code), `link_completion.rs`, `lsp-types`.

### 2026-11-08: Italic and highlight render

- Italic looked upright: Inter italics weren't bundled. Added Inter 4.1
  Italic/Medium/SemiBold/Bold Italic (same release as the others).
- `==highlight==` showed raw: CommonMark has no highlight. `core::format::highlight_spans`
  + preview turns them into `<mark>` (code, links and tags untouched).
- Editor research and decision: see Next steps #1.

### 2026-11-08: Finish Notes, Tasks, Today

- Core: `recurrence`, `quick_add`, `tags`, `project`, `format`; migration 0002
  (note_tags, projects, task recurrence/parent/project).
- Notes: three-control toolbar, grouped format menu, ⌘E toggle, indent guides
  off, content-height editor (fixes the scroll-inside-editor bug), tag filter,
  unlinked mentions with Link, daily note, import/export.
- Tasks: quick-add parsing, repeats, steps in the detail panel, project views.
- Today: rewrite with week chart and next 7 days (rows open the task).
- Toasts moved bottom-right (they covered the toolbar).
- 10 new UI tests; queried elements use `.test_support()`.

### 2026-11-08: Panels and menus

- Reusable `SidePanel` (`crates/ui/src/panel.rs`) + shared `LayoutStore`
  (`apps/desktop/src/ui_state.rs`); the notes list is the first user.
  ⌘⇧L / title-bar button toggles it. Tested headless and with real keystrokes.
- Row menus reorganized (submenus show current value, icons, right-click = same menu).
- `[[` completion highlights exactly the typed prefix.

### 2026-11-08: Links, autocomplete, task detail

- Note body is now gpui-kit `Editor` (UI font, soft wrap, no gutter) with a
  `CompletionProvider` (`views/link_completion.rs`): typing `[[` suggests note
  titles (prefix matches first) and inserts `Title]]`.
- Preview rewrites `[[links]]` to `calmfold://note/…` Markdown links
  (`core::links::to_markdown_links`); clicking opens the note or creates it.
- Title matching folds Unicode case in Rust (fixes the `lower()` review item).
- Task detail sheet (`views/task_detail.rs`): click a task or "Details…".
  Title/notes autosave, also on blur and when the sheet closes.
- 9 new tests (core links, storage suggestions, 3 UI flows).

### 2026-11-08: Foundation

- Chose GPUI via gpui-kit 0.9.0 (ADR 0001), SQLite + FTS5 (ADR 0002),
  Argon2id + XChaCha20-Poly1305 (ADR 0003). Name: Calmfold.
- Built core (notes, tasks + state machine, wiki links), storage (migrations,
  FTS5, backlinks), crypto, design system (themes, fonts, appearance, icons,
  feedback), desktop shell and screens, UI integration tests, docs.
- Bugs found by tests and fixed: app-level commands silently did nothing
  (window borrowed during dispatch, now deferred); extra Lucide icons rendered
  blank (now embedded + guarded by a test); list previews repeated the title.
- Did not code-sign anything; dev builds never touch the keychain.
- Review fixes: notes and layout now save on ⌘Q and window close; a failed
  save no longer lets the editor switch notes and drop typing; title input
  capped at 300 chars; palette runs commands after it closes and focuses its
  search on open (New note from ⌘K now focuses the title); task menu only
  offers allowed transitions (Start/Pause/Cancel/Reopen); tasks get "Delete
  forever" (confirmed) and an "Empty trash" button on the Trash tab; Upcoming
  quick-add is due tomorrow; Tasks/badges refresh after midnight; load
  failures show an error state instead of an empty list. 3 regression tests added.

### 2026-11-17: Rename the app to Ottero

- Calmfold → Ottero by Nexdark Labs everywhere user-visible and in code
  (crates, binary, bundle/exe names, updater helper, env vars, themes, preview
  link scheme, Emergency Kit, seeds, docs, AGENTS.md, vendored patch notes).
  New icon files wired into `build.rs`, `bundle-mac.sh`, `release.yml`.
- Migrations with tests: data folder Calmfold/Keepstone → Ottero and
  `calmfold.sqlite3`/`keepstone.sqlite3` → `ottero.sqlite3` (once, never
  overwriting); saved "Calmfold Light/Dark" → "Ottero Light/Dark"; old vault
  archive extensions still listed and opened.
- Updates (ADR 0009, evidence from Sparkle/Velopack source): bundle ID and
  packId kept; `--mainExe Ottero.exe` and `--packTitle Ottero`. Local Sparkle
  installed-upgrade test now goes from Calmfold-named to Ottero-named fixture
  and passes (plus tamper rejection). Windows probe updated, not yet run.
- Version bumped to 0.1.3 with a "Calmfold is now Ottero" What's new entry
  (0.1.2 is published and immutable). Ran the debug app on a throwaway folder
  (light mode, migrated theme name), and `bundle-mac.sh --debug` (Ottero name
  and icon, ad-hoc seal only).
