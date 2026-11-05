# Building release artifacts

The current release is a local-only desktop tracker. The build does not publish
anything, configure login startup, grant OS permissions, or upload signing keys.

## Local macOS build

```bash
cargo tauri build --ci --no-sign --bundles app,dmg -- --locked
```

The bundle is under `src-tauri/target/release/bundle/`. For a faster UI check:

```bash
cargo tauri build --debug --no-sign --bundles app -- --locked
DAYLOG_HOME=/absolute/path/to/disposable-data \
  src-tauri/target/debug/bundle/macos/daylog.app/Contents/MacOS/daylog
```

Keep the test process in the foreground and quit from the tray after checking:

1. Dashboard loads, filters update, and totals include the running session.
2. Log search and date filters work; an empty result shows the empty message.
3. CSV and Markdown exports open a native Save dialog and contain the selected rows.
4. Pause/resume and the exclusion blocklist update the status correctly.
5. An idle interval stops counting, and a sleep/resume gap is excluded.
6. Enable encryption, quit/relaunch, reject a wrong password, and unlock correctly.
7. Save unrelated settings while encrypted without reentering the password.
8. Confirm Clear from both the window and tray; Cancel keeps the data.
9. Close the window, reopen from the tray, then quit normally.

Unit tests cover state transitions, persistence failures, migration, password
verification, crash tails, midnight totals, and retention. Frontend tests cover
IPC wiring, status rendering, settings errors, escaping, and export selection.
Native OS permissions, idle timers, dialogs, and installers need platform smoke tests.

## Cross-platform packages

Run **Installer packages** manually from GitHub Actions. It produces artifacts
for macOS (`.app`, `.dmg`), Windows (NSIS, MSI), and Linux (`.deb`, AppImage).
Artifacts are unsigned and are not published automatically. CI separately runs
formatting, Clippy, backend tests, frontend tests, and a release executable build
on all three platforms.

GitHub artifact downloads do not preserve Unix executable permissions reliably.
Use the DMG for macOS distribution and the installers on other platforms.

## Public distribution requirements

- macOS requires a Developer ID Application certificate, signing identity, and
  Apple notarization credentials. Configure these using the Tauri CLI's signing
  environment variables in a private CI environment, remove `--no-sign`, and
  verify signing/notarization on the resulting bundle before publishing.
- Windows public distribution needs an appropriate code-signing certificate and
  a configured signing command. Verify signatures on both installer formats.
- Review the version in `Cargo.toml` and `tauri.conf.json`, run platform smoke
  tests, and publish only the reviewed artifacts.
- Automatic updates are not implemented. They would require an update host,
  signed update artifacts, and a deliberate change to the no-network policy.

Never commit signing material or passwords. Packaging automation intentionally
contains no credentials and performs no public release action.
