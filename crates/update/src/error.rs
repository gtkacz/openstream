//! The one error type every step of an update returns.

use std::path::PathBuf;

use thiserror::Error;

use crate::RELEASES_URL;

/// Why an update could not be checked, downloaded, or installed. Every variant's `Display` is
/// shown to the user as it is, so it says what happened and what state the install is left in.
#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("github: {0}")]
    Http(#[from] reqwest::Error),
    #[error("not a release tag: {0:?}")]
    Version(String),
    #[error("checksum: {0}")]
    Checksum(String),
    #[error("archive rejected: {0}")]
    Archive(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("install directory {} is not writable: {source}", .dir.display())]
    NotWritable {
        dir: PathBuf,
        source: std::io::Error,
    },
    #[error("could not replace the installed files: {source}; {}", rollback_note(*.rolled_back))]
    Apply {
        source: std::io::Error,
        rolled_back: bool,
    },
}

fn rollback_note(rolled_back: bool) -> String {
    if rolled_back {
        "the previous version was restored".to_string()
    } else {
        format!("the previous version could not be restored, reinstall from {RELEASES_URL}")
    }
}
