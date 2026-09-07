//! Whether the libraries FFmpeg's VAAPI device path loads on demand are installed.
//!
//! The shipped BtbN FFmpeg does not link libva: generated stubs `dlopen` it on the first
//! VAAPI call and abort the process when the library is missing. Checking up front turns
//! that abort into a skipped probe.

#[cfg(target_os = "linux")]
use std::ffi::CStr;

/// Everything the null-device VAAPI path can reach: libva with its DRM entry point for render
/// nodes, then the X11 entry point FFmpeg falls back to when no render node opens.
#[cfg(target_os = "linux")]
const VAAPI_LIBRARIES: &[&CStr] = &[
    c"libva.so.2",
    c"libva-drm.so.2",
    c"libdrm.so.2",
    c"libva-x11.so.2",
    c"libX11.so.6",
];

#[cfg(target_os = "linux")]
pub(crate) fn available() -> bool {
    all_loadable(VAAPI_LIBRARIES)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn available() -> bool {
    false
}

#[cfg(target_os = "linux")]
fn all_loadable(names: &[&CStr]) -> bool {
    names.iter().all(|name| {
        // SAFETY: the name is NUL-terminated and the handle passed to dlclose is the one dlopen
        // returned, checked for null first.
        let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_LAZY) };
        if handle.is_null() {
            return false;
        }
        unsafe { libc::dlclose(handle) };
        true
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::all_loadable;

    #[test]
    fn a_library_that_is_not_installed_makes_the_set_unloadable() {
        assert!(!all_loadable(&[
            c"libc.so.6",
            c"libbrp-no-such-library.so.0"
        ]));
    }

    #[test]
    fn libraries_already_in_the_process_are_loadable() {
        assert!(all_loadable(&[c"libc.so.6"]));
    }
}
