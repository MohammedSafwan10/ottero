<div align="center">
  <img src="assets/brand/png/icon-128.png" width="88" height="88" alt="Ottero icon: a curled otter forming an O">
  <h1>Ottero</h1>
  <p>Notes, tasks, calendar, habits, focus and passwords.<br>Local-first. Native. Made for macOS, Windows and Android.</p>
  <p>
    <a href="https://github.com/MohammedSafwan10/calmfold-releases/releases/latest">Download</a> ·
    <a href="CONTRIBUTING.md">Contribute</a> ·
    <a href="docs/roadmap.md">Roadmap</a> ·
    <a href="LICENSE">GPLv3</a>
  </p>
</div>

![Ottero Today dashboard in light mode, showing a schedule, habits and tasks with fictional sample data](docs/images/today-light.png)

Ottero brings everyday personal tools into one quiet workspace. Your notes and
plans live on your device. There is no required account, cloud subscription or
telemetry. Desktop is built in Rust with GPUI Kit; Android is native Kotlin
(Jetpack Compose) sharing the same Rust core, SQLite storage and E2EE sync via
UniFFI. iOS is planned next.

## Download

Get official builds from [Ottero Releases](https://github.com/MohammedSafwan10/calmfold-releases/releases/latest).

| Platform | Package |
| --- | --- |
| macOS · Apple Silicon | `Ottero-<version>-macos-arm64.zip` — extract and move Ottero.app to Applications |
| Windows · x64 | `Ottero-win-Setup.exe` — per-user installer |
| Windows · portable | `Ottero-win-Portable.zip` — extract and keep the complete folder together |
| Android · APK | `Ottero-<version>-android.apk` — universal `arm64-v8a` & `x86_64` APK (Google Play coming soon) |

These early builds are not yet Apple-notarized or Windows publisher-signed, so
the OS may warn on first installation. Update signatures are verified separately.

Starting with v0.1.2, installed builds use **Sparkle on Mac** and **Velopack on
Windows**. Install this release once manually; future versions can download in
the background and install from Settings → About. Read the release history inside
the app under **What’s new**.

## One workspace, your way

| Tool | What it does |
| --- | --- |
| **Today** | Your schedule, due tasks, habits and upcoming days, with sections you can arrange or hide |
| **Notes** | Live Markdown, wiki links and backlinks, tags, full-text search, math and diagrams |
| **Tasks** | Quick add, priorities, repeats, subtasks and projects |
| **Calendar** | Month, week, day and agenda views, recurring events, `.ics` import and offline holidays |
| **Habits** | Flexible schedules, counts, streaks and insights |
| **Focus** | Focus and break timers, linked tasks and session history |
| **Passwords** | An encrypted vault, one-time codes, password generation, recovery keys and encrypted backups |
| **Developer** | Projects, useful links, setup notes, tasks and encrypted environment sets with `.env` import/export |

Choose your visible sidebar tools, their order and your startup page. Curated
**Ottero Light (Quiet Alabaster)** and **Ottero Dark (Smoked Obsidian)** themes,
fonts and keyboard shortcuts help the workspace fit the way you work.

## Local data, clear boundaries

Personal content stays local by default; release checks contact GitHub. Optional
**Ottero Cloud** sync encrypts notes, tasks, calendar, habits, focus and
passwords end to end before they leave your device. Local notes, tasks and
calendar data on disk are **not encrypted by the app**; the password vault uses
Argon2id and XChaCha20-Poly1305. Keep a device backup and store your Emergency
Kit or vault recovery key somewhere safe.

See the [security design](docs/security.md) for the threat model and recovery
limits, or [report a vulnerability privately](SECURITY.md).

## Build and contribute

Rust is pinned in [`rust-toolchain.toml`](rust-toolchain.toml).

- **macOS:** Xcode and its Metal toolchain.
- **Windows:** MSVC Rust, Visual Studio C++ build tools and CMake.
- **Android:** Android SDK 36, NDK 28, JDK 21 and `cargo-ndk` (`./scripts/build-android-rust.sh` then `cd apps/android && ./gradlew assembleDebug`).

```sh
git clone https://github.com/MohammedSafwan10/ottero.git
cd ottero
cargo run
```

The first build can take several minutes. Debug builds use a separate **Ottero
Dev** data folder; installing a release does not automatically migrate that data.
Use `OTTERO_DATA_DIR` with a new throwaway directory for experiments.

Run `scripts/check.sh` before submitting a change (Git Bash on Windows).
The [contribution guide](CONTRIBUTING.md) covers setup, tests and pull requests.

## Project notes

[Architecture](docs/architecture.md) · [Design system](docs/design-system.md) ·
[Testing](docs/testing.md) · [Roadmap](docs/roadmap.md) ·
[Code of Conduct](CODE_OF_CONDUCT.md)

Ottero’s original source is licensed under **GPL-3.0-only**. Third-party code,
fonts and icons retain their own terms; see [NOTICE](NOTICE) and the bundled
third-party notices. The license does not grant trademark rights or imply
endorsement of a fork.

Made by **Nexdark Labs**.
