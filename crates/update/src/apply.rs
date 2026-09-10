//! Swapping the staged release into the install directory: the previous file is renamed aside,
//! never deleted, because Windows lets a running exe and its loaded DLLs be renamed but not
//! removed. A failure part-way puts every rename back.

use std::ffi::OsString;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

use crate::OLD_SUFFIX;
use crate::error::UpdateError;
use crate::install::Install;

/// A verified, extracted release waiting in its staging directory.
#[derive(Debug)]
pub struct Staged {
    pub(crate) dir: PathBuf,
    pub(crate) files: Vec<OsString>,
}

/// One completed step: `current` now holds the new file, and `previous` is where the old one went
/// (`None` when there was no old one).
struct Swap {
    current: PathBuf,
    previous: Option<PathBuf>,
}

/// Moves every staged file into place. The staging directory is removed whatever happens.
pub fn apply(staged: Staged, install: &Install) -> Result<(), UpdateError> {
    let mut swaps = Vec::new();
    let outcome = swap_all(&staged, install, &mut swaps);
    let _ = fs::remove_dir_all(&staged.dir);
    match outcome {
        Ok(()) => Ok(()),
        Err(source) => {
            let rolled_back = roll_back(swaps);
            Err(UpdateError::Apply {
                source,
                rolled_back,
            })
        }
    }
}

fn swap_all(staged: &Staged, install: &Install, swaps: &mut Vec<Swap>) -> io::Result<()> {
    for name in &staged.files {
        let current = install.dir.join(name);
        let mut previous_name = name.clone();
        previous_name.push(OLD_SUFFIX);
        let previous = install.dir.join(previous_name);
        if previous.exists() {
            fs::remove_file(&previous)?;
        }
        let had_current = current.exists();
        if had_current {
            fs::rename(&current, &previous)?;
        }
        // Recorded before the second rename so a failure there still puts `previous` back.
        swaps.push(Swap {
            current: current.clone(),
            previous: had_current.then_some(previous),
        });
        fs::rename(staged.dir.join(name), &current)?;
    }
    Ok(())
}

/// Reverses the completed swaps, newest first. Returns whether every one went back.
fn roll_back(swaps: Vec<Swap>) -> bool {
    let mut restored = true;
    for swap in swaps.into_iter().rev() {
        let result = match &swap.previous {
            Some(previous) => fs::rename(previous, &swap.current),
            None => match fs::remove_file(&swap.current) {
                Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
                other => other,
            },
        };
        if let Err(error) = result {
            tracing::error!(%error, path = %swap.current.display(), "rollback failed");
            restored = false;
        }
    }
    restored
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_install(test: &str) -> Install {
        let dir = std::env::temp_dir().join(format!("brp-apply-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Install::at(dir.join("brp")).unwrap()
    }

    fn stage(install: &Install, files: &[(&str, &[u8])], listed: &[&str]) -> Staged {
        let dir = install.dir.join(".brp-update-test");
        fs::create_dir(&dir).unwrap();
        for (name, data) in files {
            fs::write(dir.join(name), data).unwrap();
        }
        Staged {
            dir,
            files: listed.iter().map(OsString::from).collect(),
        }
    }

    fn listing(install: &Install) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&install.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn every_staged_file_replaces_its_predecessor_which_survives_as_old() {
        let install = temp_install("swap");
        fs::write(install.dir.join("brp"), "v1").unwrap();
        fs::write(install.dir.join("LICENSE"), "MIT").unwrap();
        let staged = stage(
            &install,
            &[("brp", b"v2"), ("LICENSE", b"MIT"), ("NEW.txt", b"new")],
            &["brp", "LICENSE", "NEW.txt"],
        );

        apply(staged, &install).unwrap();

        assert_eq!(fs::read(install.dir.join("brp")).unwrap(), b"v2");
        assert_eq!(fs::read(install.dir.join("brp.old")).unwrap(), b"v1");
        assert_eq!(fs::read(install.dir.join("NEW.txt")).unwrap(), b"new");
        assert_eq!(
            listing(&install),
            ["LICENSE", "LICENSE.old", "NEW.txt", "brp", "brp.old"]
        );
    }

    #[test]
    fn a_stale_old_file_is_replaced_by_the_current_one() {
        let install = temp_install("stale");
        fs::write(install.dir.join("brp"), "v2").unwrap();
        fs::write(install.dir.join("brp.old"), "v1").unwrap();
        let staged = stage(&install, &[("brp", b"v3")], &["brp"]);

        apply(staged, &install).unwrap();

        assert_eq!(fs::read(install.dir.join("brp")).unwrap(), b"v3");
        assert_eq!(fs::read(install.dir.join("brp.old")).unwrap(), b"v2");
    }

    #[test]
    fn a_failure_part_way_restores_every_file_and_reports_the_rollback() {
        let install = temp_install("rollback");
        fs::write(install.dir.join("brp"), "v1").unwrap();
        fs::write(install.dir.join("libx.so"), "lib1").unwrap();
        // `libx.so` is listed but never staged, so its rename fails after `brp` was swapped.
        let staged = stage(
            &install,
            &[("brp", b"v2"), ("NEW.txt", b"n")],
            &["brp", "NEW.txt", "libx.so"],
        );

        let error = apply(staged, &install).unwrap_err();

        assert!(
            matches!(
                error,
                UpdateError::Apply {
                    rolled_back: true,
                    ..
                }
            ),
            "{error}"
        );
        assert_eq!(fs::read(install.dir.join("brp")).unwrap(), b"v1");
        assert_eq!(fs::read(install.dir.join("libx.so")).unwrap(), b"lib1");
        assert_eq!(listing(&install), ["brp", "libx.so"]);
    }
}
