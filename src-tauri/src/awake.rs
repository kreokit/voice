//! Keeps the computer from going to sleep while a long job runs (CLAUDE.md M2 decision).
//!
//! Only idle *system* sleep is prevented; the display may still turn off, and closing a
//! laptop's lid still sleeps it. The hold ends when the [`KeepAwake`] guard is dropped.
//!
//! - **macOS:** an IOKit power assertion (`PreventUserIdleSystemSleep`), visible in
//!   `pmset -g assertions` as "KreoKit Voice: exporting". Apps holding one are also not
//!   slowed down by App Nap. IOKit and CoreFoundation are system frameworks (macOS 10.6+),
//!   so nothing new is bundled.
//! - **Windows:** `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` from
//!   kernel32, which applies to the calling thread; the guard is not `Send`, so it is
//!   released on the same thread.
//! - **Linux:** nothing yet.

use std::marker::PhantomData;

/// Prevents idle sleep until dropped. Holding it is best-effort: if the system refuses,
/// the job just runs without it.
pub struct KeepAwake {
    #[cfg(target_os = "macos")]
    assertion: Option<u32>,
    /// Not `Send`: Windows ties the request to the thread that made it.
    _thread: PhantomData<*const ()>,
}

impl KeepAwake {
    pub fn new(reason: &str) -> Self {
        #[cfg(target_os = "macos")]
        {
            Self {
                assertion: mac::hold(reason),
                _thread: PhantomData,
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = reason;
            #[cfg(windows)]
            win::hold();
            Self {
                _thread: PhantomData,
            }
        }
    }

    /// Whether the system accepted the request (only known on macOS).
    #[cfg(all(test, target_os = "macos"))]
    fn is_held(&self) -> bool {
        self.assertion.is_some()
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(id) = self.assertion.take() {
            mac::release(id);
        }
        #[cfg(windows)]
        win::release();
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{CString, c_char, c_void};

    type CFStringRef = *const c_void;
    const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8
    const LEVEL_ON: u32 = 255; // kIOPMAssertionLevelOn

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            text: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            kind: CFStringRef,
            level: u32,
            name: CFStringRef,
            id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    fn cf_string(text: &str) -> Option<CFStringRef> {
        let c = CString::new(text).ok()?;
        // SAFETY: `c` is a valid NUL-terminated string for the duration of the call.
        let s = unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) };
        (!s.is_null()).then_some(s)
    }

    pub fn hold(reason: &str) -> Option<u32> {
        let kind = cf_string("PreventUserIdleSystemSleep")?;
        let name = cf_string(&format!("KreoKit Voice: {reason}"));
        let mut id = 0u32;
        // SAFETY: both strings are live CFStrings; `id` is a valid out pointer. The
        // assertion keeps its own copies, so the strings are released right after.
        let result = unsafe {
            let r = match name {
                Some(name) => IOPMAssertionCreateWithName(kind, LEVEL_ON, name, &mut id),
                None => -1,
            };
            if let Some(name) = name {
                CFRelease(name);
            }
            CFRelease(kind);
            r
        };
        (result == 0).then_some(id)
    }

    pub fn release(id: u32) {
        // SAFETY: `id` came from a successful IOPMAssertionCreateWithName.
        unsafe {
            IOPMAssertionRelease(id);
        }
    }
}

#[cfg(windows)]
mod win {
    const ES_CONTINUOUS: u32 = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
    }

    pub fn hold() {
        // SAFETY: plain flag call with no pointers.
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
        }
    }

    pub fn release() {
        // SAFETY: as above; ES_CONTINUOUS alone clears this thread's request.
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_and_releases() {
        let guard = KeepAwake::new("test");
        #[cfg(target_os = "macos")]
        {
            assert!(guard.is_held());
            let listed = std::process::Command::new("pmset")
                .args(["-g", "assertions"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains("KreoKit Voice: test"))
                .unwrap_or(true); // pmset missing: nothing to compare
            assert!(listed, "assertion not listed by pmset");
        }
        drop(guard);
    }
}
