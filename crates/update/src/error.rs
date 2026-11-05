//! The one error type every step of an update returns.

use std::path::PathBuf;

use thiserror::Error;

use crate::RELEASES_URL;

/// Why an update could not be checked, downloaded, or installed. Every variant's `Display` is
/// shown to the user as it is, so it says what happened and what state the install is left in.
#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("Could not download from GitHub. Check your connection and try again. ({0})")]
    Http(#[from] reqwest::Error),
    #[error("not a release tag: {0:?}")]
    Version(String),
    #[error("The download failed its integrity check, so nothing was installed. Try again. ({0})")]
    Checksum(String),
    #[error("The download is not a valid release, so nothing was installed. Try again. ({0})")]
    Archive(String),
    #[error("No release is published for this platform. Build from source to update.")]
    Unpublished,
    #[error(
        "Could not finish the download, so nothing was installed. Check your disk space and try again. ({0})"
    )]
    Io(#[from] std::io::Error),
    #[error("Could not write to {}, so nothing was installed. Download the update from {} instead. ({source})", .dir.display(), RELEASES_URL)]
    NotWritable {
        dir: PathBuf,
        source: std::io::Error,
    },
    #[error("Could not replace the installed files. {} ({source})", rollback_note(*.rolled_back))]
    Apply {
        source: std::io::Error,
        rolled_back: bool,
    },
}

fn rollback_note(rolled_back: bool) -> String {
    if rolled_back {
        "The previous version was restored, so try again.".to_string()
    } else {
        format!("The previous version could not be restored, so reinstall from {RELEASES_URL}.")
    }
}
