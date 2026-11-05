# daylog

**A transparent, privacy-respecting activity logger that shows where your time goes.**

![Rust](https://img.shields.io/badge/Rust-stable_1.90%2B-orange?logo=rust)
![Tauri](https://img.shields.io/badge/Tauri-v2-24C8DB?logo=tauri)
![License: MIT](https://img.shields.io/badge/License-MIT-green)

> ### 🔒 Privacy first
> **daylog does NOT capture keystrokes, typed text, passwords, clipboard contents, or screen contents.**
> It records only three things: the **active application name**, the **window title**, and **how long** that window was focused.
> There is no keyboard hook anywhere in the codebase. Everything stays on your machine, with no network access and no telemetry.
> daylog is a self-tracking tool you run on your own computer. It shows up in the process list, has a normal window, and has a tray icon. It is not a keylogger and not a surveillance tool.

---

## ✨ Features

- **Idle detection**: stops counting after a configurable OS idle threshold (5 minutes by default), with sleep-gap protection.
- **Crash recovery**: sessions checkpoint every 30 seconds; status and queries use cached history.
- **Activity tracking**: records the focused app, window title, and time spent, written to a local log file.
- **Analytics**: per-app and per-day breakdowns of where your time went.
- **Search and filter**: by app, date range, or keyword.
- **Export**: CSV and Markdown.
- **Configurable log path**: keep the log wherever you like.
- **Optional encryption**: AES-256-GCM with an Argon2id-derived key. History migrates when switching modes; your password is never persisted.
- **Exclusion blocklist**: logging pauses automatically while a blocked app or window title is focused (password managers, banking, private documents).
- **Start on login**: toggle via `tauri-plugin-autostart`.
- **Auto-purge**: atomically delete expired events at startup and hourly while running.
- **Pause / resume** and **clear logs** at any time.
- **System tray** with quick actions.
- **Tiny frontend**: plain HTML/CSS/JS in `ui/`, no framework and no npm build step.

## 🖼️ Screenshots

Screenshots can be added under [`docs/`](docs/). Suggested files:

| View | File |
| --- | --- |
| Dashboard | `docs/screenshot-dashboard.png` |
| Search and filters | `docs/screenshot-search.png` |
| Settings | `docs/screenshot-settings.png` |

<!-- Once added: ![Dashboard](docs/screenshot-dashboard.png) -->

## 🛠️ Install / Build

daylog is built from source with Rust and the Tauri v2 CLI. See **[docs/BUILD.md](docs/BUILD.md)** for prerequisites and per-OS instructions (macOS, Windows, Linux).

```bash
git clone https://github.com/ninad-k/daylog.git
cd daylog
cargo install tauri-cli --version "^2"
cargo tauri build
```

## 🚀 Quick Start

```bash
cargo tauri dev
```

1. Launch the app. On macOS, grant the permissions described in [docs/PERMISSIONS.md](docs/PERMISSIONS.md), then restart it.
2. Use your computer normally. daylog logs the active app, window title, and duration.
3. Open the dashboard to see per-app and per-day totals.
4. Open Settings to add sensitive apps to the exclusion blocklist, and consider turning on encryption.
5. Export to CSV or Markdown whenever you want a report.

## 🧩 How it works

daylog is split into cleanly separated layers:

```
┌───────────────────────────────┐
│  ui/        static HTML/CSS/JS│  presentation only
├───────────────────────────────┤
│  commands   Tauri IPC         │  thin bridge: UI <-> core
├───────────────────────────────┤
│  core       state + logic     │  polling loop, sessions, analytics,
│                               │  blocklist, purge, pause/resume
├──────────────┬────────────────┤
│  capture/    │  storage/      │
│  sensing     │  persistence   │
│  (active-win │  Store trait:  │
│   -pos-rs)   │  JSONL | AES-GCM│
└──────────────┴────────────────┘
```

- **`capture/`** asks the OS for the focused app name and window title via the [`active-win-pos-rs`](https://crates.io/crates/active-win-pos-rs) crate. No `rdev`, no keyboard hooks.
- **`storage/`** persists events behind a `Store` trait with two interchangeable backends: plaintext **JSONL** (default, one JSON event per line) and optional **AES-256-GCM** encryption with an Argon2id-derived key.
- **`core`** owns application state and logic: the polling loop, merging samples into sessions, the exclusion blocklist, analytics, auto-purge, and pause/resume.
- **`commands`** exposes core functionality to the frontend over Tauri IPC.
- **`ui/`** renders the dashboard with plain files.

Because layers only talk to their neighbours through narrow interfaces, you can swap a storage backend or capture source without touching the rest.

## ⚙️ Configuration

Settings are stored in the OS config directory and edited from the Settings screen.

| Setting | Description | Default |
| --- | --- | --- |
| Log path | Where the activity log is written | OS data dir, e.g. `~/Library/Application Support/com.ninadk.daylog/activity.log` on macOS |
| Encryption | AES-256-GCM with an Argon2id-derived key; password never stored | Off (plaintext JSONL) |
| Exclusion blocklist | Apps or window-title patterns that pause logging while focused | Empty |
| Start on login | Launch daylog automatically at login | Off |
| Auto-purge days | Delete events older than N days | Off |
| Poll interval | How often the active window is sampled (250–60000 ms) | `1500` |
| Idle threshold | Stop counting after this many seconds without input; blank disables | `300` |

## 🔐 Permissions

| OS | Requirement |
| --- | --- |
| macOS | **Screen Recording** permission to read window titles; may also prompt for **Accessibility** |
| Windows | None. SmartScreen or antivirus may warn about an unsigned binary |
| Linux | Works on **X11**. On **Wayland**, window titles may be unavailable (compositor-dependent) |

daylog needs no keyboard-hook permission because it does not hook the keyboard. Full details and step-by-step guides: **[docs/PERMISSIONS.md](docs/PERMISSIONS.md)**.

## 🛡️ Security & Privacy

The full statement is in **[docs/PRIVACY.md](docs/PRIVACY.md)**. Honest highlights:

- **The plaintext log is readable by anything that can read the file.** Any process running as your user, or anyone with access to your disk, can read it. **Enable encryption** to protect it.
- **Window titles can be sensitive.** A title may contain a document name, a webpage title, or a chat contact. Use the **exclusion blocklist** and/or **encryption** for anything you would not want in a log.
- **Antivirus tools generally will not flag daylog**, because there is no keyboard hook. An **unsigned binary may still trigger SmartScreen (Windows) or Gatekeeper (macOS)** prompts. That is about code signing, not behavior.
- **Local only.** No network calls, no telemetry, no accounts.

## 🗺️ Roadmap

The core tracker, idle detection, encryption migration, and reliability work are implemented. Richer analytics and signed distribution remain future enhancements. See **[ROADMAP.md](ROADMAP.md)**.

## 🤝 Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first. Note that adding keystroke or typed-text capture is an explicit non-goal and will never be merged.

## 📄 License

MIT © 2026 Ninad K. See [LICENSE](LICENSE).

Repository: <https://github.com/ninad-k/daylog>

## Reliability and development checks

Run backend and frontend checks with:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
node --test tests/ui.test.cjs
```

Node 22+ is used only for frontend tests; running and packaging the app needs no Node toolchain.
The manual **Installer packages** workflow builds unsigned macOS, Windows, and Linux artifacts without publishing them. See [release instructions](docs/RELEASING.md).

For isolated testing or portable storage, set `DAYLOG_HOME` to an absolute directory. This puts settings, the process lock, and the default log there. Without it, the normal OS directories are used.

Queries include the current session. Analytics clips sessions to the selected time range and splits time across local calendar days. The log table filters by session start, and its row limit also applies to exports. Checkpoints reuse session IDs; the latest complete revision wins when reopening a log. A crash may lose up to 30 seconds, plus one sampling interval. A damaged complete record produces a visible error; an incomplete final record is discarded during recovery.
