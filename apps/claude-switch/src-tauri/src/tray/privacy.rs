//! Whether anything is capturing the screen (a Zoom or Meet share, a screen
//! recording, macOS Screen Sharing), so the menu bar can hide usage from an
//! audience.
//!
//! macOS gives apps no public "someone is capturing" call. The window server
//! keeps the flag its own capture indicator reads, `SLSIsScreenWatcherPresent`
//! (`CGSIsScreenWatcherPresent` before SkyLight); it is looked up at run time,
//! so a macOS without it falls back to the public session dictionary, which
//! still reports Screen Sharing and remote sessions. OpenUsage detects it the
//! same way (MIT).

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void, CStr};
    use std::sync::OnceLock;

    extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    /// dlfcn.h's `RTLD_DEFAULT`: search every image loaded, SkyLight among them.
    const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

    type IsWatcherPresent = unsafe extern "C" fn() -> bool;

    fn is_watcher_present() -> Option<IsWatcherPresent> {
        static FUNCTION: OnceLock<Option<usize>> = OnceLock::new();
        let address = *FUNCTION.get_or_init(|| {
            [c"SLSIsScreenWatcherPresent", c"CGSIsScreenWatcherPresent"]
                .iter()
                .map(|name: &&CStr| {
                    // SAFETY: dlsym with RTLD_DEFAULT and a NUL-terminated name
                    // only looks the symbol up.
                    unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) }
                })
                .find(|address| !address.is_null())
                .map(|address| address as usize)
        });
        // SAFETY: both spellings are `bool (void)` functions in the window
        // server's client library.
        address.map(|address| unsafe { std::mem::transmute::<usize, IsWatcherPresent>(address) })
    }

    pub fn screen_captured() -> bool {
        match is_watcher_present() {
            // SAFETY: a no-argument query of the window server.
            Some(present) => unsafe { present() },
            None => session_screen_shared(),
        }
    }

    /// The public fallback: Screen Sharing and remote sessions, not in-app captures.
    fn session_screen_shared() -> bool {
        use objc2_core_foundation::{CFBoolean, CFDictionary, CFRetained, CFString};

        extern "C" {
            fn CGSessionCopyCurrentDictionary() -> Option<CFRetained<CFDictionary>>;
        }
        // SAFETY: returns a retained dictionary, or null outside a GUI session.
        let Some(session) = (unsafe { CGSessionCopyCurrentDictionary() }) else {
            return false;
        };
        let key = CFString::from_static_str("CGSSessionScreenIsShared");
        // SAFETY: the key is a CFString; the value, when present, a CFBoolean.
        let value = unsafe { session.value(&*key as *const CFString as *const c_void) };
        if value.is_null() {
            return false;
        }
        // SAFETY: as above, the session dictionary stores booleans under this key.
        let boolean = unsafe { &*(value as *const CFBoolean) };
        boolean.value()
    }
}

#[cfg(target_os = "macos")]
pub use macos::screen_captured;

#[cfg(not(target_os = "macos"))]
pub fn screen_captured() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asking_never_fails() {
        // Whatever the answer on this machine, asking must not crash or hang.
        let _ = screen_captured();
    }
}
