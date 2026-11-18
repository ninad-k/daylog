//! Read only the OS's elapsed idle time. No input hooks, event streams, or
//! key/button identities are collected. Unsupported sessions return `None`.

use std::time::Duration;

pub fn duration() -> Option<Duration> {
    platform::duration()
}

#[cfg(target_os = "windows")]
mod platform {
    use super::Duration;

    #[repr(C)]
    struct LastInputInfo {
        size: u32,
        tick: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTickCount() -> u32;
    }

    pub fn duration() -> Option<Duration> {
        let mut info = LastInputInfo {
            size: std::mem::size_of::<LastInputInfo>() as u32,
            tick: 0,
        };
        // LASTINPUTINFO.dwTime uses the same wrapping 32-bit millisecond clock
        // as GetTickCount, including on 64-bit Windows.
        unsafe {
            if GetLastInputInfo(&mut info) == 0 {
                None
            } else {
                Some(elapsed(GetTickCount(), info.tick))
            }
        }
    }

    fn elapsed(now: u32, last: u32) -> Duration {
        Duration::from_millis(now.wrapping_sub(last) as u64)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn elapsed_handles_clock_wraparound() {
            assert_eq!(elapsed(100, u32::MAX - 49), Duration::from_millis(150));
            assert_eq!(elapsed(10_000, 9_000), Duration::from_secs(1));
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::Duration;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    }

    pub fn duration() -> Option<Duration> {
        // kCGEventSourceStateCombinedSessionState = 0; kCGAnyInputEventType = ~0.
        let seconds = unsafe { CGEventSourceSecondsSinceLastEventType(0, u32::MAX) };
        if seconds.is_finite() && seconds >= 0.0 && seconds < u64::MAX as f64 {
            Some(Duration::from_secs_f64(seconds))
        } else {
            None
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::Duration;
    use std::ffi::{c_char, c_int, c_ulong, c_void};

    #[repr(C)]
    struct ScreenSaverInfo {
        window: c_ulong,
        state: c_int,
        kind: c_int,
        til_or_since: c_ulong,
        idle: c_ulong,
        event_mask: c_ulong,
    }

    #[link(name = "X11")]
    extern "C" {
        fn XOpenDisplay(name: *const c_char) -> *mut c_void;
        fn XDefaultRootWindow(display: *mut c_void) -> c_ulong;
        fn XCloseDisplay(display: *mut c_void) -> c_int;
        fn XFree(data: *mut c_void) -> c_int;
    }
    #[link(name = "Xss")]
    extern "C" {
        fn XScreenSaverQueryExtension(
            display: *mut c_void,
            event_base: *mut c_int,
            error_base: *mut c_int,
        ) -> c_int;
        fn XScreenSaverAllocInfo() -> *mut ScreenSaverInfo;
        fn XScreenSaverQueryInfo(
            display: *mut c_void,
            drawable: c_ulong,
            info: *mut ScreenSaverInfo,
        ) -> c_int;
    }

    pub fn duration() -> Option<Duration> {
        // XWayland's clock covers X clients only, so reporting it as desktop
        // idle time would silently discard time spent in native Wayland apps.
        if std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland")
        {
            return None;
        }
        unsafe {
            let display = XOpenDisplay(std::ptr::null());
            if display.is_null() {
                return None;
            }
            let (mut event, mut error) = (0, 0);
            let result = if XScreenSaverQueryExtension(display, &mut event, &mut error) == 0 {
                None
            } else {
                let info = XScreenSaverAllocInfo();
                if info.is_null() {
                    None
                } else {
                    let result =
                        if XScreenSaverQueryInfo(display, XDefaultRootWindow(display), info) != 0 {
                            // C unsigned long differs between 32/64-bit X11.
                            #[allow(clippy::unnecessary_cast)]
                            let milliseconds = (*info).idle as u64;
                            Some(Duration::from_millis(milliseconds))
                        } else {
                            None
                        };
                    XFree(info.cast());
                    result
                }
            };
            XCloseDisplay(display);
            result
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod platform {
    use super::Duration;

    pub fn duration() -> Option<Duration> {
        None
    }
}
