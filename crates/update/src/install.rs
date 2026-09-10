//! Where the running binary lives and whether that directory is an extracted release.

use std::fs;
use std::io;
use std::path::PathBuf;

use crate::error::UpdateError;
use crate::{OLD_SUFFIX, RELEASE_MARKER, STAGING_PREFIX};

/// The directory the running binary was started from and the binary's path, taken once at startup:
/// after an update has renamed the running file, `current_exe` on Linux resolves to the `.old`
/// inode, so the path must be captured before any swap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub dir: PathBuf,
    pub exe: PathBuf,
}

impl Install {
    /// The install the running process belongs to. Errors when the platform will not name the
    /// running binary, which leaves updates noticeable but not applicable.
    pub fn current() -> Result<Self, UpdateError> {
        Self::at(std::env::current_exe()?)
    }

    /// The install is the directory `exe` sits in.
    pub fn at(exe: PathBuf) -> Result<Self, UpdateError> {
        let dir = exe
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| {
                UpdateError::Io(io::Error::other(format!(
                    "{} has no parent directory",
                    exe.display()
                )))
            })?
            .to_path_buf();
        Ok(Self { dir, exe })
    }

    /// True when the directory was produced by a staging script, so replacing its files is what
    /// an update means. A `cargo build` output has no such marker and is never touched.
    pub fn is_release_layout(&self) -> bool {
        self.dir.join(RELEASE_MARKER).is_file()
    }
}

/// Removes the previous version's `.old` files and any staging directory an earlier run left.
/// Best effort: on Windows the process that was just replaced may still be exiting, in which
/// case its files are removed by the launch after this one.
///
/// Callers gate this on [`Install::is_release_layout`]: elsewhere a matching name is someone
/// else's file, not a leftover of ours.
pub fn cleanup_stale(install: &Install) {
    let entries = match fs::read_dir(&install.dir) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::debug!(%error, dir = %install.dir.display(), "install directory not listed");
            return;
        }
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let path = entry.path();
        let result = if name.ends_with(OLD_SUFFIX) && path.is_file() {
            fs::remove_file(&path)
        } else if name.starts_with(STAGING_PREFIX) && path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            continue;
        };
        if let Err(error) = result {
            tracing::debug!(%error, path = %path.display(), "leftover from an earlier update kept");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_install(test: &str) -> Install {
        let dir = std::env::temp_dir().join(format!("brp-install-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Install::at(dir.join("brp")).unwrap()
    }

    #[test]
    fn the_install_is_the_directory_of_the_exe() {
        let install = Install::at(PathBuf::from("/opt/brp-0.5.0/brp")).unwrap();
        assert_eq!(install.dir, PathBuf::from("/opt/brp-0.5.0"));
        assert_eq!(install.exe, PathBuf::from("/opt/brp-0.5.0/brp"));
        assert!(Install::at(PathBuf::from("/")).is_err());
    }

    #[test]
    fn the_ffmpeg_licence_beside_the_exe_marks_a_release_layout() {
        let install = temp_install("layout");
        assert!(!install.is_release_layout());
        fs::write(install.dir.join(RELEASE_MARKER), "LGPL").unwrap();
        assert!(install.is_release_layout());
    }

    #[test]
    fn cleanup_removes_old_files_and_staging_directories_and_nothing_else() {
        let install = temp_install("cleanup");
        fs::write(install.dir.join("brp"), "new").unwrap();
        fs::write(install.dir.join(format!("brp{OLD_SUFFIX}")), "old").unwrap();
        fs::write(
            install.dir.join(format!("libavcodec.so.62{OLD_SUFFIX}")),
            "old",
        )
        .unwrap();
        let staging = install.dir.join(format!("{STAGING_PREFIX}0.6.0"));
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("partial"), "x").unwrap();
        fs::write(install.dir.join("LICENSE"), "MIT").unwrap();

        cleanup_stale(&install);

        let mut names: Vec<String> = fs::read_dir(&install.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["LICENSE", "brp"]);
    }
}
