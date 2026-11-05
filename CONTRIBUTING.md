# Contributing to daylog

Thanks for your interest in daylog. Contributions of all sizes are welcome.

## 🚫 Non-goal: keystroke or text capture

daylog records only the active app name, window title, and duration. **Pull requests must never add keystroke, typed-text, clipboard, password, screenshot, or screen-content capture.** No keyboard hooks, no `rdev`, no input monitoring. This is a deliberate design choice and an explicit non-goal. Such PRs will be closed.

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
| `capture/` | Sensing: active app and window title (`active-win-pos-rs`) |
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
  cd src-tauri
  cargo fmt --all
  cargo clippy --all-targets -- -D warnings
  cargo test
  cd ..
  node --test tests/ui.test.cjs
  ```
- Keep privacy in mind: no network calls, no telemetry.

## Pull requests

1. Open an issue first for larger changes.
2. Branch from `main`, keep PRs focused.
3. Describe what changed and why, and which OS you tested on.
4. Update docs if behavior or settings change.

By contributing, you agree your contributions are licensed under the [MIT License](LICENSE).
