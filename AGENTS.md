# AGENTS.md

Guidance for coding agents (and humans) working on **Ottero**, a local-first,
all-in-one personal app (notes, tasks, later a password vault, calendar,
habits, finance) written in Rust with a GPUI desktop UI.

Read this file, then [`HANDOFF.md`](HANDOFF.md) for the current state, then the
`AGENTS.md` of the crate you are changing. The closest `AGENTS.md` wins.

## Commands

```bash
source "$HOME/.cargo/env"          # if cargo is not on PATH
scripts/check.sh                   # fmt --check + clippy -D warnings + all tests (run before every commit)
scripts/clean-build.sh             # weekly: drop build files unused for 7 days (keeps target/ ~4-6 GB)
cargo run                          # run the desktop app (debug data: "Ottero Dev" folder)
scripts/dev.sh [--sample]          # run + auto-rebuild/relaunch on save (needs watchexec)
OTTERO_DATA_DIR=/tmp/ks cargo run   # run against a throwaway data folder
cargo test -p ottero-desktop --test ui   # headless UI integration tests only
scripts/screenshot.sh /tmp/k.png   # capture ONLY the Ottero window (never the full screen)
swift scripts/click.swift X Y      # real click in the window (points = screenshot px / 2 on Retina)
```

Toolchain is pinned in `rust-toolchain.toml` (Rust 1.99.1). The first build
takes ~3 minutes; GPUI dependencies are optimised even in debug (see
`[profile.dev.package."*"]`) so the app stays smooth during development.

## Workspace map

| Path | Crate | Responsibility | May depend on |
| --- | --- | --- | --- |
| `crates/core` | `ottero-core` | Domain models and rules (Note, Task, IDs, wiki links, state machines). No IO, no clock except `clock.rs`. | nothing internal |
| `crates/storage` | `ottero-storage` | SQLite schema, migrations, FTS5 search, repositories. | core |
| `crates/crypto` | `ottero-crypto` | Argon2id KDF, XChaCha20-Poly1305 envelope, zeroizing keys. For the vault. | nothing internal |
| `crates/ui` | `ottero-ui` | Design system: themes, fonts, appearance settings, icon assets, reusable components, feedback (confirm/undo/error). | gpui-kit |
| `apps/desktop` | `ottero-desktop` | The GPUI app: shell, sidebar, screens, actions, command palette. | all of the above |

Dependency direction is one-way: `desktop → ui/storage/crypto → core`. Never
import the UI from core/storage/crypto. Mobile and web shells will reuse
core/storage/crypto later, so keep them UI-free.

## Quality bar (read first)

Do the best version, not the quick one. The owner reviews the running app.

- **Finish the whole thing.** A feature is UI + keyboard path + empty/error
  states + persistence + tests + docs. No placeholders, no "TODO later", no
  half-wired buttons.
- **Look it up, don't guess.** Read the real gpui-kit source/docs before using
  an API; read how polished apps (Linear, Obsidian, Things, Bear, Raycast)
  solve the same UX problem before designing.
- **Fix root causes.** If a test or the app shows a bug, find why. Don't hide
  it, special-case it, or loosen the test.
- **One source of truth.** Shortcuts, nav, panels, themes, icons and menus are
  data tables; extend the table instead of hard-coding a second copy.
- **Verify like a user.** Run `scripts/check.sh`, run the app (`scripts/dev.sh`),
  look at the screen in light and dark. If real clicking is unreliable, prove
  the flow with a UI integration test instead of skipping it.
- **Keep it calm.** Fewer, better-organized controls: overflow menus over
  icon rows, shortcuts for power use, consistent menu order (open → change →
  status → destructive).
- **Report honestly.** Say what was verified, what wasn't, and what's next.

## Hard rules

1. **Never invent a GPUI / gpui-kit API.** Look up the real signature in
   `~/.cargo/registry/src/*/gpui-component-0.9.0`, `gpui-base-0.9.0`,
   `gpui-pre-0.3.11`, or the vendored skills in `.kiro/skills/gpui-kit`.
   Read `.kiro/skills/gpui-kit-design-guides` before any UI change.
2. **One UI dependency.** Depend on `gpui-kit` only (it pins a `gpui-pre`
   snapshot). Never add `gpui` directly. Upgrade GPUI only by upgrading
   gpui-kit, then rerun every test and look at the app. Sole exception: the
   note editor (`zorite-editor`, `ratex-gpui`, ADR 0004), pinned with `=` and
   only while it resolves to the same `gpui-pre` (`cargo tree -d -i gpui-pre`).
3. **No hard-coded colors** in feature code. Use `cx.theme()` tokens. Raw hex
   lives only in `crates/ui/themes/ottero.json`.
4. **Views never touch SQLite.** They call `Library` methods
   (`apps/desktop/src/library.rs`), which apply domain rules, persist, and emit
   `LibraryEvent`.
5. **Domain rules live in core** and take `now` as a parameter. Use
   `ottero_core::clock::now()` at the call site.
6. **Destructive actions** follow `ottero_ui::feedback`: reversible → run
   and offer Undo; permanent → `confirm_destructive`; errors → `error_toast`.
7. **Every command is an action** (`apps/desktop/src/actions.rs`), handled at
   app level in `shell::register_commands`. Add it to the palette list and
   the shortcuts table when it has a shortcut.
8. **New Lucide icon?** Add it to `AppIcons` in `crates/ui/src/assets.rs`.
   A test fails if you forget (unlisted icons render blank).
9. **Migrations are append-only.** Never edit a shipped `.sql` file; add
   `000N_*.sql` and append it in `crates/storage/src/schema.rs`.
10. **Security.** No secrets, note bodies, or passwords in logs or error
    messages. No `unsafe` (forbidden workspace-wide). Crypto changes need
    tests and must follow `crates/crypto/AGENTS.md`.
11. **Do not code-sign** with any identity found on the machine. Development
    builds must not touch the system keychain (see `docs/security.md`).
12. **Screenshots** only via `scripts/screenshot.sh` (Ottero window only).
13. **Public repo boundary.** This repo (`ottero`) is public and open-source
    for the local-first desktop app only. Never commit cloud backend internals,
    payment/email vendor details, hosting/VPS specs, pricing/monetization
    strategy, internal admin routes, personal emails, or local machine paths
    here (including in `HANDOFF.md`, `changelog.json`, `docs/`, or comments).
    Cloud backend, billing, and business strategy docs belong strictly in the
    private `ottero-cloud` repo.

## Definition of done

- `scripts/check.sh` passes (fmt, clippy `-D warnings`, all tests).
- Behaviour has a test at the right layer: pure logic → unit test in the
  crate; persistence → storage test; UI flow → `apps/desktop/tests/ui.rs`.
- You ran the app and looked at the changed screen in light and dark mode.
- Empty, error, and loading states exist for anything new.
- `HANDOFF.md` is updated (current state, what changed, next steps).
- If a plan in `docs/plans/` changed, update its status checklist.

## Docs

- `docs/architecture.md`: layers, data flow, startup, state machines.
- `docs/design-system.md`: theme, typography, spacing, components, feedback rules.
- `docs/security.md`: threat model, vault crypto design, keychain policy.
- `docs/testing.md`: what to test where and how.
- `docs/roadmap.md` and `docs/plans/*.md`: one plan per feature, in order.
- `docs/decisions/*.md`: short ADRs for choices that are expensive to reverse.

## Style

- Rust 2024 edition, `cargo fmt` defaults, Clippy pedantic-lite via workspace lints.
- `unwrap`/`expect` are warned in production code; use `?`, `context`, or a
  logged fallback. Allowed in tests.
- Names: spell `Context` out; `cx` is reserved for GPUI contexts. Avoid `Kind`
  suffixes. Stable, domain-derived `ElementId`s for list rows (`note-{id}`).
- Comments explain *why*. Module docs describe the state machine when there is one.
- Commit messages: imperative, short subject (`Add note backlinks panel`).
