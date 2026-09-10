//! Registering the binary as the handler for `brp://` links, so a click in a browser opens the
//! participant window. There is no installer: the window registers itself on every launch,
//! pointing at whatever binary is running, so a moved or upgraded binary stays reachable. A
//! failure is a warning, never a reason to keep the window from opening.

use std::io;

#[cfg(target_os = "linux")]
pub use linux::{DESKTOP_FILE, desktop_entry, install};

/// Registers the running executable for `brp://` links on a detached thread. `xdg-mime` can be
/// slow or absent and a registry can be locked by policy, so nothing waits on the outcome and
/// every problem is logged at warn.
pub fn register_scheme_handler() {
    let spawned = std::thread::Builder::new()
        .name("scheme-handler".into())
        .spawn(|| {
            if let Err(error) = register() {
                tracing::warn!(%error, "could not register the brp:// link handler");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start the brp:// link handler registration");
    }
}

#[cfg(target_os = "linux")]
fn register() -> io::Result<()> {
    linux::register()
}

#[cfg(windows)]
fn register() -> io::Result<()> {
    windows::register()
}

#[cfg(not(any(target_os = "linux", windows)))]
fn register() -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs;
    use std::io;
    use std::path::Path;
    use std::process::Command;

    /// The handler-only desktop entry, under the user's applications directory.
    pub const DESKTOP_FILE: &str = "brp.desktop";
    const SCHEME_MIME: &str = "x-scheme-handler/brp";

    /// The desktop entry that hands `brp://` links to `exe`. `NoDisplay` keeps it out of
    /// launchers: started from a menu without a URL, `brp join` is a usage error.
    pub fn desktop_entry(exe: &Path) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=brp\nComment=Peer-to-peer screen sharing\n\
             Exec={} join %u\nTerminal=false\nNoDisplay=true\nMimeType={SCHEME_MIME};\n",
            quote_exec(exe)
        )
    }

    /// Quotes a path for an `Exec` key. The desktop entry specification applies two rules in
    /// turn: the quoting rule wants `"`, `` ` ``, `$`, and `\` preceded by a backslash inside the
    /// quotes, and the string rule then wants each backslash itself doubled, so the file carries
    /// two backslashes before those characters and four for a literal backslash.
    fn quote_exec(path: &Path) -> String {
        let mut quoted = String::from("\"");
        for c in path.to_string_lossy().chars() {
            match c {
                '"' | '`' | '$' => quoted.push_str("\\\\"),
                '\\' => quoted.push_str("\\\\\\"),
                _ => {}
            }
            quoted.push(c);
        }
        quoted.push('"');
        quoted
    }

    /// Writes the entry into `dir` when its content differs from what is there, creating the
    /// directory if needed. Returns whether it wrote.
    pub fn install(dir: &Path, exe: &Path) -> io::Result<bool> {
        let path = dir.join(DESKTOP_FILE);
        let entry = desktop_entry(exe);
        if fs::read_to_string(&path).is_ok_and(|current| current == entry) {
            return Ok(false);
        }
        fs::create_dir_all(dir)?;
        fs::write(&path, entry)?;
        Ok(true)
    }

    pub fn register() -> io::Result<()> {
        let exe = std::env::current_exe()?;
        let dirs = directories::BaseDirs::new()
            .ok_or_else(|| io::Error::other("no home directory for the desktop entry"))?;
        let dir = dirs.data_local_dir().join("applications");
        let written = install(&dir, &exe)?;
        tracing::debug!(path = %dir.join(DESKTOP_FILE).display(), written, "desktop entry checked");
        // The default is claimed on every launch, not only after a write: another application or
        // a fresh desktop session can reset mimeapps.list while the entry itself is unchanged.
        let status = Command::new("xdg-mime")
            .args(["default", DESKTOP_FILE, SCHEME_MIME])
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "xdg-mime default exited with {status}"
            )));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use std::path::PathBuf;

        use super::*;

        fn temp_dir(name: &str) -> PathBuf {
            let dir = std::env::temp_dir()
                .join(format!("brp-handler-test-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            dir
        }

        #[test]
        fn the_entry_is_a_hidden_handler_for_the_scheme() {
            let entry = desktop_entry(Path::new("/opt/brp/brp"));
            assert!(entry.starts_with("[Desktop Entry]\n"), "{entry}");
            assert!(
                entry.contains("\nExec=\"/opt/brp/brp\" join %u\n"),
                "{entry}"
            );
            assert!(entry.contains("\nNoDisplay=true\n"), "{entry}");
            assert!(
                entry.contains("\nMimeType=x-scheme-handler/brp;\n"),
                "{entry}"
            );
            assert!(entry.contains("\nTerminal=false\n"), "{entry}");
        }

        #[test]
        fn the_exec_path_is_quoted_and_escaped() {
            let entry = desktop_entry(Path::new("/opt/my apps/brp$1"));
            assert!(
                entry.contains("\nExec=\"/opt/my apps/brp\\\\$1\" join %u\n"),
                "{entry}"
            );
        }

        #[test]
        fn install_writes_once_and_again_when_the_exe_moves() {
            let dir = temp_dir("install");
            let path = dir.join(DESKTOP_FILE);
            assert!(install(&dir, Path::new("/a/brp")).unwrap());
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                desktop_entry(Path::new("/a/brp"))
            );
            assert!(!install(&dir, Path::new("/a/brp")).unwrap());
            assert!(install(&dir, Path::new("/b/brp")).unwrap());
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                desktop_entry(Path::new("/b/brp"))
            );
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr;

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegSetValueExW,
    };

    /// The per-user class for the scheme; no elevation is needed under HKEY_CURRENT_USER.
    const CLASS_KEY: &str = r"Software\Classes\brp";

    /// One string value under the class: `subkey` relative to the class (empty for the class
    /// itself), `name` of the value (`None` for the default value), and its data.
    pub struct ClassValue {
        pub subkey: &'static str,
        pub name: Option<&'static str>,
        pub data: String,
    }

    /// Everything the shell needs to hand `brp://` links to `exe`: what the scheme is, that it is
    /// a URL protocol, an icon, and the command. Pure, so the shape is checked without a registry.
    pub fn class_values(exe: &Path) -> Vec<ClassValue> {
        let exe = exe.display();
        vec![
            ClassValue {
                subkey: "",
                name: None,
                data: "URL:brp".to_string(),
            },
            ClassValue {
                subkey: "",
                name: Some("URL Protocol"),
                data: String::new(),
            },
            ClassValue {
                subkey: "DefaultIcon",
                name: None,
                data: format!("\"{exe}\",0"),
            },
            ClassValue {
                subkey: r"shell\open\command",
                name: None,
                data: format!("\"{exe}\" join \"%1\""),
            },
        ]
    }

    pub fn register() -> io::Result<()> {
        let exe = std::env::current_exe()?;
        for value in class_values(&exe) {
            let path = if value.subkey.is_empty() {
                CLASS_KEY.to_string()
            } else {
                format!("{CLASS_KEY}\\{}", value.subkey)
            };
            set_string(&path, value.name, &value.data)?;
        }
        Ok(())
    }

    fn wide(text: &str) -> Vec<u16> {
        OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// Creates `path` under HKEY_CURRENT_USER if needed and sets one REG_SZ value; `None` names
    /// the key's default value. Overwrites, so the same call is the update path.
    fn set_string(path: &str, name: Option<&str>, data: &str) -> io::Result<()> {
        let path = wide(path);
        let name = name.map(wide);
        let data = wide(data);
        let mut key: HKEY = ptr::null_mut();
        // SAFETY: every pointer is to a live NUL-terminated buffer, or null where the API allows
        // it; the key is closed before every return that follows its creation.
        unsafe {
            let created = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                ptr::null(),
                &mut key,
                ptr::null_mut(),
            );
            if created != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(created as i32));
            }
            // cbData counts bytes including the terminating NUL, as REG_SZ requires.
            let set = RegSetValueExW(
                key,
                name.as_ref().map_or(ptr::null(), |n| n.as_ptr()),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            );
            RegCloseKey(key);
            if set != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(set as i32));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_command_quotes_the_exe_and_passes_the_url() {
            let values = class_values(Path::new(r"C:\Program Files\brp\brp.exe"));
            let command = values
                .iter()
                .find(|v| v.subkey == r"shell\open\command")
                .expect("command value");
            assert_eq!(command.data, r#""C:\Program Files\brp\brp.exe" join "%1""#);
            assert!(values.iter().any(|v| v.name == Some("URL Protocol")));
        }
    }
}
