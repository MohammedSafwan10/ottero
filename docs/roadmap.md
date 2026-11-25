# Roadmap

One plan per feature in `docs/plans/`. Build in order; each phase ends with a
usable app, passing `scripts/check.sh`, and an updated `HANDOFF.md`.

| # | Plan | Status |
| --- | --- | --- |
| 01 | [Foundation: workspace, design system, shell, settings](plans/01-foundation.md) | Done |
| 02 | [Notes](plans/02-notes.md) | Done (live-preview editor, links, find, Mermaid, math, spell check) |
| 03 | [Tasks](plans/03-tasks.md) | Done (repeats, subtasks, projects) |
| 04 | [Today dashboard](plans/04-today.md) | Done (customisable sections) |
| 05 | [Password vault](plans/05-vault.md) | Done (recovery, backups, templates, TOTP) |
| 06 | [Optional E2EE Cloud Sync](plans/06-sync.md) | Done (desktop client, protocol, live WS push, unified Cloud Passwords) |
| 07 | [Calendar, habits, journal](plans/07-calendar-habits.md) | Calendar and habits done (holidays, ICS import, birthdays); journal open |
| 08 | [Finance and subscriptions](plans/08-finance.md) | Not started |
| 09 | [Packaging and releases](plans/09-packaging.md) | Done (icon, macOS/Windows bundles, Sparkle + Velopack auto-updates; notarization/Authenticode open) |
| 10 | [Mobile](plans/10-mobile.md) | Not started |
| 11 | [Developer workspace](plans/11-developer.md) | Projects, linked setup notes/tasks and encrypted environment sets done; broader developer tools deferred |
| 12 | [Workspace customization](plans/12-workspace-customization.md) | Sidebar visibility/order, inline More, startup choice and organized settings implemented |
| 13 | [Production updates](plans/13-production-updates.md) | Live (Sparkle 2 on macOS, Velopack on Windows, signed catalog + immutable releases) |

Cross-cutting ideas to schedule later: global quick-capture hotkey, links
between any objects (note ↔ task ↔ vault item), import from Obsidian/markdown
folders, plugin system (WASM), reminders/notifications (needs a signed build).
