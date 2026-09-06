//! The participant's key pair, created on first run in the platform config directory.
use crate::error::AppError;
use data_encoding::HEXLOWER;
use directories::ProjectDirs;
use iroh::SecretKey;
use std::fs;
use std::path::Path;
use std::str::FromStr;

pub fn load_or_create() -> Result<SecretKey, AppError> {
    let dirs = ProjectDirs::from("", "", "brp")
        .ok_or_else(|| AppError::Identity("no home directory to store the identity in".into()))?;
    load_or_create_at(&dirs.config_dir().join("identity.key"))
}
pub fn load_or_create_at(path: &Path) -> Result<SecretKey, AppError> {
    if let Ok(text) = fs::read_to_string(path) {
        return SecretKey::from_str(text.trim()).map_err(|e| {
            AppError::Identity(format!("{} is not a valid key: {e}", path.display()))
        });
    }
    let key = SecretKey::generate();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    write_private(path, HEXLOWER.encode(&key.to_bytes()).as_bytes())?;
    tracing::info!(path = %path.display(), id = %key.public().fmt_short(), "created a new identity");
    Ok(key)
}
#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
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
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
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
        let a = load_or_create_at(&path).unwrap();
        let b = load_or_create_at(&path).unwrap();
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
