//! The participant's key pair, created on first run in the platform config directory.
use crate::error::AppError;
use data_encoding::HEXLOWER;
use directories::ProjectDirs;
use iroh::SecretKey;
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::Path;
use std::str::FromStr;

/// The key this process runs as, and the lock that makes it this process's alone. Hold the lock for
/// as long as the key is in use.
pub fn load_or_create() -> Result<(SecretKey, Option<File>), AppError> {
    let dirs = ProjectDirs::from("", "", "brp").ok_or_else(|| {
        AppError::Identity(
            "No home directory to store the identity in. Run OpenStream as a user with a \
                 home directory."
                .into(),
        )
    })?;
    load_or_create_at(&dirs.config_dir().join("identity.key"))
}

/// Two processes on one key fight over it: a relay delivers to only one connection per key, and in
/// the same room they count as one member. So a second window takes a key of its own for this run.
/// The lock is a separate file because Windows would refuse the second process a read of a locked
/// key.
pub fn load_or_create_at(path: &Path) -> Result<(SecretKey, Option<File>), AppError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|error| not_saved(path, error))?;
    }
    let claim = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))
        .and_then(|lock| {
            lock.try_lock()?;
            Ok(lock)
        });
    match claim {
        Ok(lock) => Ok((saved_or_new(path)?, Some(lock))),
        Err(error) if error.kind() == ErrorKind::WouldBlock => {
            tracing::info!("another window holds the saved identity; using a temporary one");
            Ok((SecretKey::generate(), None))
        }
        // A file system without locks must not cost the user their saved identity.
        Err(error) => {
            tracing::warn!(%error, "could not lock the identity; using it unlocked");
            Ok((saved_or_new(path)?, None))
        }
    }
}

fn not_saved(path: &Path, error: std::io::Error) -> AppError {
    AppError::Identity(format!(
        "Could not save {} ({error}). Check that you can write to that folder, then start \
                 OpenStream again.",
        path.display()
    ))
}

fn saved_or_new(path: &Path) -> Result<SecretKey, AppError> {
    if let Ok(text) = fs::read_to_string(path) {
        return SecretKey::from_str(text.trim()).map_err(|e| {
            AppError::Identity(format!(
                "{} is not a valid identity key ({e}). Delete it to create a new identity, then \
                 start OpenStream again.",
                path.display()
            ))
        });
    }
    let key = SecretKey::generate();
    write_private(path, HEXLOWER.encode(&key.to_bytes()).as_bytes())
        .map_err(|error| not_saved(path, error))?;
    tracing::info!(path = %path.display(), id = %key.public().fmt_short(), "created a new identity");
    Ok(key)
}
#[cfg(unix)]
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}
#[cfg(not(unix))]
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    fn temp_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("brp-identity-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("identity.key")
    }
    #[test]
    fn creates_then_reloads_the_same_key() {
        let path = temp_path("reload");
        let (a, _) = load_or_create_at(&path).unwrap();
        let (b, _) = load_or_create_at(&path).unwrap();
        assert_eq!(a.to_bytes(), b.to_bytes());
        assert_eq!(fs::read_to_string(path).unwrap().trim().len(), 64);
    }
    #[cfg(unix)]
    #[test]
    fn key_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_path("mode");
        load_or_create_at(&path).unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    #[test]
    fn a_second_window_gets_its_own_key_until_the_first_lets_go() {
        let path = temp_path("second-window");
        let (first, lock) = load_or_create_at(&path).unwrap();
        assert!(lock.is_some());
        let (second, none) = load_or_create_at(&path).unwrap();
        assert!(none.is_none(), "only one process can hold the lock");
        assert_ne!(first.to_bytes(), second.to_bytes());
        drop(lock);
        let (again, _) = load_or_create_at(&path).unwrap();
        assert_eq!(first.to_bytes(), again.to_bytes());
    }
    #[test]
    fn a_key_that_cannot_be_saved_names_the_file_and_what_to_do() {
        let path = temp_path("unwritable");
        // A file where the folder should be fails the same way on every platform.
        fs::write(path.parent().unwrap(), "").unwrap();
        let error = load_or_create_at(&path).unwrap_err().to_string();
        assert!(error.contains("identity.key"), "{error}");
        assert!(error.contains("Check that you can write"), "{error}");
    }
    #[test]
    fn corrupt_key_file_is_reported_not_overwritten() {
        let path = temp_path("corrupt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "not hex").unwrap();
        assert!(matches!(
            load_or_create_at(&path),
            Err(AppError::Identity(_))
        ));
        assert_eq!(fs::read_to_string(path).unwrap(), "not hex");
    }
}
