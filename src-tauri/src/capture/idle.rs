//! Read only the OS idle timer. No input events or input contents are captured.
#[cfg(target_os = "macos")]
pub fn seconds() -> Option<u64> {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event: u32) -> f64;
    }
    // Combined session state, any input event. This is an elapsed-time query.
    let value = unsafe { CGEventSourceSecondsSinceLastEventType(0, u32::MAX) };
    (value.is_finite() && value >= 0.0).then_some(value as u64)
}

#[cfg(target_os = "windows")]
pub fn seconds() -> Option<u64> {
    #[repr(C)]
    struct LastInputInfo {
        size: u32,
        time: u32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTickCount() -> u32;
    }
    let mut info = LastInputInfo {
        size: std::mem::size_of::<LastInputInfo>() as u32,
        time: 0,
    };
    unsafe {
        if GetLastInputInfo(&mut info) == 0 {
            None
        } else {
            Some(GetTickCount().wrapping_sub(info.time) as u64 / 1000)
        }
    }
}

#[cfg(target_os = "linux")]
pub fn seconds() -> Option<u64> {
    use libloading::{Library, Symbol};
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
    // XScreenSaver is optional; unavailable timers are surfaced in status.
    // The display is opened, used and closed on this capture thread only.
    unsafe {
        let x = Library::new("libX11.so.6").ok()?;
        let ss = Library::new("libXss.so.1").ok()?;
        let open: Symbol<unsafe extern "C" fn(*const c_char) -> *mut c_void> =
            x.get(b"XOpenDisplay").ok()?;
        let close: Symbol<unsafe extern "C" fn(*mut c_void) -> c_int> =
            x.get(b"XCloseDisplay").ok()?;
        let root: Symbol<unsafe extern "C" fn(*mut c_void) -> c_ulong> =
            x.get(b"XDefaultRootWindow").ok()?;
        let query: Symbol<
            unsafe extern "C" fn(*mut c_void, c_ulong, *mut ScreenSaverInfo) -> c_int,
        > = ss.get(b"XScreenSaverQueryInfo").ok()?;
        let display = open(std::ptr::null());
        if display.is_null() {
            return None;
        }
        let mut info: ScreenSaverInfo = std::mem::zeroed();
        let success = query(display, root(display), &mut info);
        close(display);
        #[cfg(target_pointer_width = "64")]
        let milliseconds = info.idle;
        #[cfg(target_pointer_width = "32")]
        let milliseconds = u64::from(info.idle);
        (success != 0).then_some(milliseconds / 1000)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn seconds() -> Option<u64> {
    None
}
