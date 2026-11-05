# Building and running daylog

daylog is a Rust + Tauri v2 app. The Rust crate lives in `src-tauri/`; the frontend is plain static files in `ui/` with no npm build step.

## Prerequisites

- **Rust** stable, 1.90 or newer: <https://rustup.rs>
- **Tauri v2 CLI**:
  ```bash
  cargo install tauri-cli --version "^2"
  ```
- **Platform webview dependencies** (see below per OS).

Verify:

```bash
rustc --version
cargo tauri --version
```

## macOS

1. Install Xcode Command Line Tools:
   ```bash
   xcode-select --install
   ```
2. Install Rust and the Tauri CLI (above).
3. Run from the project root:
   ```bash
   cargo tauri dev      # development
   cargo tauri build    # release bundle
   ```
4. Grant permissions on first run. See [PERMISSIONS.md](PERMISSIONS.md). Window titles will be empty until **Screen Recording** is enabled and the app restarted.

## Windows

1. Install Rust with the MSVC toolchain, and the "Desktop development with C++" workload from Visual Studio Build Tools.
2. **WebView2** is bundled with modern Windows 10/11. If missing, install the Evergreen runtime from Microsoft.
3. Run from the project root:
   ```powershell
   cargo tauri dev
   cargo tauri build
   ```
4. No special permissions are needed. SmartScreen or antivirus may warn about an unsigned binary.

## Linux (Ubuntu / Debian)

Install system dependencies:

```bash
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Then:

```bash
cargo tauri dev
cargo tauri build
```

Other distributions need the equivalent webkit2gtk 4.1, OpenSSL, and appindicator/ayatana packages. Active-window detection works on **X11**; on **Wayland** it is restricted (see troubleshooting).

## Development vs release

| | Command | Notes |
| --- | --- | --- |
| Dev | `cargo tauri dev` | Run from project root. Debug build; faster compile, slower runtime. |
| Release | `cargo tauri build` | Optimized build plus installers/bundles. |

## Where the built bundles land

Output is under `src-tauri/target/release/` and its `bundle/` subfolder:

| OS | Location |
| --- | --- |
| macOS | `src-tauri/target/release/bundle/macos/daylog.app` and `bundle/dmg/` |
| Windows | `src-tauri/target/release/bundle/msi/` and `bundle/nsis/` |
| Linux | `src-tauri/target/release/bundle/deb/`, `bundle/appimage/`, and `bundle/rpm/` |

The exact set depends on `bundle.targets` in the Tauri configuration. The raw executable is in `src-tauri/target/release/`.

## Troubleshooting

**Empty window titles, or titles equal to the app name (macOS).**
Grant **Screen Recording** (and possibly **Accessibility**) to daylog in System Settings, then fully quit and relaunch. When running with `cargo tauri dev`, the permission may need to be granted to your terminal app or the dev binary. Details in [PERMISSIONS.md](PERMISSIONS.md).

**No window titles on Linux under Wayland.**
Wayland deliberately restricts one client from inspecting another's windows. Detection may return nothing or only partial data depending on the compositor. Log in to an X11 session to use full tracking.

**`cargo tauri` not found.**
Run `cargo install tauri-cli --version "^2"` and ensure `~/.cargo/bin` is on your `PATH`.

**Linux build fails on `webkit2gtk` / `javascriptcoregtk`.**
Install `libwebkit2gtk-4.1-dev` (not the older 4.0 package).

**Tray icon missing on Linux.**
Install `libayatana-appindicator3-dev` at build time and ensure your desktop environment supports status-notifier tray icons (GNOME needs an extension).

**Windows SmartScreen warning.**
The binary is unsigned. Choose "More info" then "Run anyway" if you built it yourself.

**macOS Gatekeeper blocks the app.**
For a locally built, unsigned app, right-click the app and choose Open, or allow it under System Settings, Privacy & Security.

## Isolated smoke test

```bash
DAYLOG_HOME=/absolute/path/to/test-data cargo tauri dev
```

Use a disposable directory to keep testing separate from your normal history.
On Linux, install the runtime package `libxss1` for X11 idle detection. The app reports when the idle timer is unavailable; on Wayland, both focus and idle information depend on compositor support.

See [RELEASING.md](RELEASING.md) for installer builds and signing requirements.
