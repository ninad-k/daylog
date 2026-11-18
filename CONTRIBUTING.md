# Contributing to daylog

Thanks for your interest in daylog. Contributions of all sizes are welcome.

## Text recording and privacy boundaries

Activity tracking records the active app name, window title, and duration by default. Optional Windows text recording must remain **off by default**, require **explicit selected-app or all-supported-app scope consent** and **encrypted, unlocked storage**, and retain visible enabled/recording indicators. Pause, idle, and exclusions must stop it. Password-marked or inaccessible fields and recognized terminal/password-manager apps must remain excluded. Unsupported or uncertain control identity must fail closed. Never present these checks as a guarantee that ordinary fields cannot contain secrets.

All-supported-app mode is an explicit, default-off choice covering current and future apps; it must preserve all field, process, pause, exclusion, and encryption checks. Do not add clipboard, screenshot, screen-content, hidden, or consent-bypassing input capture. Do not send recorded text through exports, summaries, emails, update requests, or telemetry. Plaintext stores must reject recorded text. Preserve the action that deletes only recorded text while retaining activity times, and test storage, opt-in, control-eligibility, and failure paths when changing this feature. See [docs/PRIVACY.md](docs/PRIVACY.md) and [docs/PERMISSIONS.md](docs/PERMISSIONS.md).

## Setting up

Follow [docs/BUILD.md](docs/BUILD.md) for prerequisites and per-OS setup. Then:

```bash
git clone https://github.com/ninad-k/daylog.git
cd daylog
cargo tauri dev
```

On macOS you will need to grant Screen Recording; see [docs/PERMISSIONS.md](docs/PERMISSIONS.md).

## Architecture

Layers are cleanly separated. Keep it that way.

| Layer | Responsibility |
| --- | --- |
| `capture/` | Active app/title, OS elapsed idle time, and gated optional Windows text recording |
| `storage/` | Persistence behind the `Store` trait (plaintext JSONL, AES-256-GCM) |
| `core` | State and logic: polling, sessions, analytics, blocklist, purge |
| `commands` | Tauri IPC bridge between UI and core |
| `ui/` | Static HTML/CSS/JS, no framework, no npm build step |

## Conventions

- **Keep layers separate.** `capture/` must not know about storage; `storage/` must not know about the UI; the UI talks only through `commands`.
- **New storage backends** implement the `Store` trait rather than special-casing callers.
- **Comment the why, not the what.** Explain non-obvious decisions, platform quirks, and ordering constraints. Do not restate the code.
- **Frontend stays plain.** No bundlers or frameworks.
- **Format and lint** before pushing:
  ```bash
  cargo fmt --all
  cargo clippy --all-targets -- -D warnings
  cargo test
  ```
- Network activity belongs only to explicit optional summary-email/update features. No telemetry or recorded-text uploads.

## Pull requests

1. Open an issue first for larger changes.
2. Branch from `main`, keep PRs focused.
3. Describe what changed and why, and which OS you tested on.
4. Update docs if behavior or settings change.

By contributing, you agree your contributions are licensed under the [MIT License](LICENSE).
