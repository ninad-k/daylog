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
  libxdo-dev libxss-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Then:

```bash
cargo tauri dev
cargo tauri build
```

Other distributions need the equivalent webkit2gtk 4.1, OpenSSL, and appindicator/ayatana packages. Active-window detection works on **X11**; on **Wayland** it is restricted (see troubleshooting).

Idle detection uses Windows `GetLastInputInfo`, macOS Quartz elapsed idle time,
or the X11 Screen Saver extension (`libXss`). It reads elapsed time only and
does not install keyboard or mouse hooks. Wayland sessions report idle
detection as unavailable because an XWayland clock does not cover native
Wayland applications.

## Development vs release

| | Command | Notes |
| --- | --- | --- |
| Dev | `cargo tauri dev` | Run from project root. Debug build; faster compile, slower runtime. |
| Release | `cargo tauri build` | Optimized build plus installers/bundles. |

For isolated native UI testing, debug builds accept a `DAYLOG_DEV_CONFIG`
environment variable containing the full path to a separate configuration
file. Seed that file with a separate absolute `log_path` before launching;
otherwise the normal default log path still applies. For example in PowerShell:

```powershell
$devProfile = Join-Path $env:TEMP 'daylog-dev'
New-Item -ItemType Directory -Path $devProfile -Force
$env:DAYLOG_DEV_CONFIG = Join-Path $devProfile 'config.json'
@{ log_path = (Join-Path $devProfile 'activity.log') } | ConvertTo-Json | Set-Content -LiteralPath $env:DAYLOG_DEV_CONFIG
cargo tauri dev
```

Other settings use their normal defaults. Release builds ignore this variable.

## Where the built bundles land

Output is under `src-tauri/target/release/` and its `bundle/` subfolder:

| OS | Location |
| --- | --- |
| macOS | `src-tauri/target/release/bundle/macos/daylog.app` and `bundle/dmg/` |
| Windows | `src-tauri/target/release/bundle/msi/` and `bundle/nsis/` |
| Linux | `src-tauri/target/release/bundle/deb/`, `bundle/appimage/`, and `bundle/rpm/` |

The exact set depends on `bundle.targets` in the Tauri configuration. The raw executable is in `src-tauri/target/release/`.

## Release installers and signing

The `Release installers` GitHub Actions workflow builds a universal macOS app
and DMG, Windows x64 MSI and NSIS installers, and Linux x64 AppImage and Debian
packages. Push a version tag matching `src-tauri/tauri.conf.json`, such as
`v0.2.2`, or run the workflow manually. It creates a **draft** release for
review; publishing the release is a separate maintainer action. No signing
certificates are included in the repository.

Set these GitHub Actions secrets for authenticated distribution:

| Platform | Secrets |
| --- | --- |
| macOS Developer ID signing | `APPLE_CERTIFICATE` (base64 P12), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` |
| macOS notarization | `APPLE_ID`, `APPLE_PASSWORD` (app-specific password), `APPLE_TEAM_ID` |
| Windows Authenticode signing | `WINDOWS_CERTIFICATE` (base64 PFX), `WINDOWS_CERTIFICATE_PASSWORD` |
| Signed updater bundles | `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` |

Without OS certificates, Windows builds are unsigned and macOS uses ad-hoc
signing. Those artifacts are useful for testing but are not notarized or
signed with a publisher identity. The workflow does not invent an identity
or bypass signing validation. A valid Developer ID certificate and Apple
Developer account are needed for notarized macOS releases. See Tauri's
[macOS signing guide](https://v2.tauri.app/distribute/sign/macos/) and
[Windows signing guide](https://v2.tauri.app/distribute/sign/windows/).

## Configure signed updates

Update signing is separate from OS signing. Generate and retain a Tauri
signing key pair on a trusted machine:

```bash
cargo tauri signer generate -w /path/to/private/daylog.key
```

Store the private key's contents in the `TAURI_SIGNING_PRIVATE_KEY` secret,
and its password in `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Set the matching
public key's contents in the **repository variable**
`TAURI_UPDATER_PUBLIC_KEY`. Keep the private key securely backed up outside
the repository. Losing it prevents installed clients that trust that key
from accepting future updates.

The release workflow injects the public key and a feed URL into the packaged
app, enables signed updater artifacts, and creates the merged `latest.json`
release asset. The default feed is
`https://github.com/OWNER/REPO/releases/latest/download/latest.json` for the
current repository. Set the optional repository variable
`TAURI_UPDATER_ENDPOINT` to use another HTTPS feed. Standard unauthenticated
GitHub release feeds require a public repository. A signed build is rejected
if its public key is missing. Ordinary local builds have no update feed;
users can still build and run the app normally.

Publish the reviewed draft release before checking for updates. Clients
only accept update bundles with valid signatures from the embedded public
key. Update checks are controlled in Settings; tracking does not upload
activity logs to the update service. See the
[Tauri updater documentation](https://v2.tauri.app/plugin/updater/).

## winget manifest

Every Windows release job uploads a `winget-manifests` workflow artifact.
It contains a version manifest, installer manifest, and English metadata
manifest for `NinadK.daylog`, using the SHA-256 of the actual generated MSI.
After publishing the GitHub release, validate and submit that manifest to
the [winget community repository](https://github.com/microsoft/winget-pkgs).
This repository does not automatically publish a package to winget.

To generate the manifests locally from a built installer:

```powershell
python packaging/winget.py src-tauri/target/release/bundle/msi/daylog_0.2.2_x64_en-US.msi --version 0.2.2
winget validate --manifest dist/winget/0.2.2
```

Keep the installer filename consistent with the release asset. If the
release URL or package identifier changes, regenerate the manifest before
submitting it. The Windows release builds use the standard Tauri MSI format;
the manifest generator targets that x64, machine-wide installer.

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
