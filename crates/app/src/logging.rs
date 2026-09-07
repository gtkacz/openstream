//! Tracing to stderr and to a file in the config directory, plus a panic hook that logs before
//! the process dies. The Windows build is a GUI-subsystem program with no console of its own, so
//! stderr alone loses everything a crashed run had to say.

use std::fs::{self, File};
use std::io;
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use directories::ProjectDirs;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

/// Beside `settings.toml` and `identity.key` in the platform config directory.
pub const LOG_FILE: &str = "brp.log";
/// The run before this one, kept because the first thing anyone does after a crash is relaunch.
pub const PREVIOUS_LOG_FILE: &str = "brp.log.1";

/// Installs the subscriber and the panic hook, and records what build this is. Returns the log's
/// path, or `None` when no file could be opened and only stderr carries the log.
pub fn init() -> Option<PathBuf> {
    install(config_dir().and_then(|dir| open_log_in(&dir)))
}

/// `init` against a directory of the caller's choosing, so a test can read what a panic wrote.
pub fn init_in(dir: &Path) -> Option<PathBuf> {
    install(open_log_in(dir))
}

fn install(opened: Result<(PathBuf, Arc<File>), String>) -> Option<PathBuf> {
    let (path, to_file) = match &opened {
        // Each event is one unbuffered write to the file, so a hard crash keeps its last lines.
        Ok((path, file)) => (
            Some(path.clone()),
            Some(fmt::layer().with_ansi(false).with_writer(file.clone())),
        ),
        Err(_) => (None, None),
    };
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().with_writer(io::stderr))
        .with(to_file)
        .init();
    install_panic_hook();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        "brp starting"
    );
    match &opened {
        Ok((path, _)) => tracing::info!(path = %path.display(), "logging to file"),
        Err(error) => tracing::warn!(%error, "no log file; a crash will leave no trace"),
    }
    path
}

fn config_dir() -> Result<PathBuf, String> {
    let dirs = ProjectDirs::from("", "", "brp").ok_or("no home directory to log in")?;
    Ok(dirs.config_dir().to_path_buf())
}

/// Moves the previous run's log aside and opens a fresh one.
fn open_log_in(dir: &Path) -> Result<(PathBuf, Arc<File>), String> {
    fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let path = dir.join(LOG_FILE);
    rotate(&path);
    let file = File::create(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok((path, Arc::new(file)))
}

/// Keeps one previous run, because a crashed run is read after the next launch has started. A
/// rename that fails is not worth refusing to log over.
fn rotate(path: &Path) {
    if path.exists() {
        let _ = fs::rename(path, path.with_file_name(PREVIOUS_LOG_FILE));
    }
}

/// Logs the panic with its location and a backtrace, then hands over to the default hook so a
/// terminal run still gets its usual message on stderr.
fn install_panic_hook() {
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        // Forced because a shipped build runs without RUST_BACKTRACE, and a crash report with no
        // trace in it is what made the first one undiagnosable.
        let backtrace = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current();
        let location = info
            .location()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "unknown".into());
        tracing::error!(
            thread = thread.name().unwrap_or("unnamed"),
            %location,
            message = info.payload_as_str().unwrap_or("<non-string panic payload>"),
            "panicked\n{backtrace}"
        );
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brp-logging-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rotating_keeps_the_last_run_and_drops_the_one_before_it() {
        let dir = temp_dir("rotate");
        let log = dir.join(LOG_FILE);
        let previous = dir.join(PREVIOUS_LOG_FILE);

        fs::write(&log, "first run").unwrap();
        rotate(&log);
        assert!(!log.exists(), "the current log is moved aside, not copied");
        assert_eq!(fs::read_to_string(&previous).unwrap(), "first run");

        fs::write(&log, "second run").unwrap();
        rotate(&log);
        assert_eq!(fs::read_to_string(&previous).unwrap(), "second run");
    }

    #[test]
    fn rotating_a_first_run_leaves_no_previous_log() {
        let dir = temp_dir("first-run");
        rotate(&dir.join(LOG_FILE));
        assert!(!dir.join(PREVIOUS_LOG_FILE).exists());
    }
}
