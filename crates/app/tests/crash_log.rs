//! The panic hook's promise: a run that dies leaves its own explanation behind.

use std::fs;
use std::panic;

#[test]
fn a_panic_lands_in_the_log_file() {
    let dir = std::env::temp_dir().join(format!("brp-crash-log-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let path = brp_app::logging::init_in(&dir).expect("a log file under the temp directory");

    let outcome = panic::catch_unwind(|| panic!("boom from the test"));
    assert!(outcome.is_err());

    let log = fs::read_to_string(&path).unwrap();
    assert!(log.contains("panicked"), "{log}");
    assert!(log.contains("boom from the test"), "{log}");
    assert!(log.contains("crash_log.rs"), "no panic location in:\n{log}");
    assert!(log.contains("panicking"), "no backtrace frames in:\n{log}");
    let _ = fs::remove_dir_all(&dir);
}
