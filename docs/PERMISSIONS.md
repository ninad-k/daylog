# Permissions

daylog reads the **active application name** and **window title**. Operating systems gate that information differently. This page explains what you need on each platform and why.

**daylog does not hook the keyboard, so it needs no keyboard-hook or input-monitoring permission.** It does not capture keystrokes, typed text, passwords, clipboard, or screen contents.

## macOS

Modern macOS gates window *titles* behind the **Screen Recording** permission. daylog does not record your screen or take screenshots. The permission is simply what macOS requires to expose other apps' window titles. Detection may also prompt for **Accessibility**.

### Screen Recording

1. Open **System Settings → Privacy & Security → Screen Recording**.
2. Enable the toggle next to **daylog**. If it is not listed, launch daylog once so macOS shows the prompt, or click **+** and add it.
3. Quit daylog completely (including the tray icon) and **restart** it.

### Accessibility (if prompted)

1. Open **System Settings → Privacy & Security → Accessibility**.
2. Enable **daylog**.
3. Restart the app.

### Notes

- Without Screen Recording, titles may come back empty or as the app name only. App names still work.
- When running `cargo tauri dev`, macOS may attribute the permission to your terminal app (Terminal, iTerm, your IDE) or the dev binary. Enable whichever entry appears.
- After rebuilding an unsigned binary, macOS may treat it as a new app and ask again. Remove the old entry, re-add it, and restart.

## Windows

Works out of the box. No special permission is required.

- **SmartScreen** may show "Windows protected your PC" for an unsigned binary. This reflects missing code signing, not behavior. Choose **More info → Run anyway** if you built it yourself.
- Antivirus tools generally do not flag daylog because it installs no keyboard hook. An unsigned binary may still get a generic reputation warning.
- Elevated (administrator) windows may not be readable from a non-elevated process.

## Linux

Active-window detection works on **X11**.

On **Wayland**, a client is not allowed to inspect other clients' windows by design. Results are compositor-dependent: window titles may be missing, or detection may not work at all. This is a Wayland limitation, not a daylog bug.

Options:

- Log in to an **X11 session** from your display manager for full functionality.
- Where your desktop runs XWayland, windows of X11 apps may still be reported, while native Wayland windows may not.

Tray icons require `libayatana-appindicator3` and a desktop environment that supports status-notifier icons.

## Summary

| OS | Required | Optional / may prompt | Caveat |
| --- | --- | --- | --- |
| macOS | Screen Recording (for titles) | Accessibility | Restart app after granting |
| Windows | None | None | Unsigned binary may trigger SmartScreen |
| Linux | None | None | Wayland may omit titles; X11 recommended |
