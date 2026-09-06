//! On Windows the binary is a GUI-subsystem program so a double-click opens no console. Started
//! from a terminal with arguments, it borrows that terminal's console so `publish`, `--help`, and
//! errors still print. The shell does not wait for a GUI process, so output may follow the prompt.

/// Attaches to the parent console when there are arguments and a parent console exists. Must run
/// before anything prints. A no-op on other platforms and for a bare double-click.
pub fn attach_parent_console() {
    #[cfg(windows)]
    {
        use std::fs::OpenOptions;
        use std::os::windows::io::IntoRawHandle;

        use windows_sys::Win32::System::Console::{
            ATTACH_PARENT_PROCESS, AttachConsole, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
        };

        if std::env::args_os().len() < 2 {
            return;
        }
        // SAFETY: plain Win32 calls with constant arguments; failure only means there is no parent
        // console, in which case output has nowhere to go anyway.
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                return;
            }
            // A GUI process attached late has no standard handles; open the console's output
            // device and install it for both streams. The files are leaked on purpose so the
            // handles stay valid for the life of the process.
            for slot in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
                if let Ok(file) = OpenOptions::new().write(true).open("CONOUT$") {
                    SetStdHandle(slot, file.into_raw_handle());
                }
            }
        }
    }
}
