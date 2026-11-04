//! Runs the refresh the in-app update performs on an `OpenStream.app` whose `Contents/MacOS` was
//! just swapped, so the sequence can be checked on a Mac without publishing a release:
//!
//! ```sh
//! cargo run -p brp --example bundle_refresh -- /path/to/OpenStream.app 1.3.0
//! ```

#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(bundle), Some(version)) = (args.next(), args.next()) else {
        eprintln!("usage: bundle_refresh <OpenStream.app> <version>");
        return std::process::ExitCode::FAILURE;
    };
    let exe = std::path::Path::new(&bundle).join("Contents/MacOS/brp");
    let install = match brp_update::Install::at(exe) {
        Ok(install) => install,
        Err(error) => {
            eprintln!("error: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    match brp_app::bundle::refresh_after_update(&install, &version) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("bundle_refresh only runs on macOS");
}
