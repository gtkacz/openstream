//! Registering the binary as the handler for `brp://` links, so a click in a browser opens the
//! participant window. On macOS the bundle's Info.plist claims the scheme and the link arrives as
//! an Apple Event (`macos_links`). There is no installer: the window registers itself on every launch,
//! pointing at whatever binary is running, so a moved or upgraded binary stays reachable. A
//! failure is a warning, never a reason to keep the window from opening.

use std::io;

#[cfg(target_os = "linux")]
pub use linux::{APP_ID, DESKTOP_FILE, desktop_entry, install};

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

#[cfg(target_os = "macos")]
fn register() -> io::Result<()> {
    macos::register()
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
fn register() -> io::Result<()> {
    Ok(())
}

/// macOS learns the scheme from the bundle's Info.plist. Registering the running bundle on every
/// launch keeps LaunchServices pointed at the copy the user actually runs after a move or an
/// update; a binary outside a bundle cannot claim a scheme at all.
#[cfg(target_os = "macos")]
mod macos {
    use std::io;

    use objc2_core_foundation::{CFString, CFURL, CFURLPathStyle};

    use crate::bundle::bundle_root;

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        /// `LSRegisterURL(CFURLRef inURL, Boolean inUpdate)`.
        fn LSRegisterURL(url: &CFURL, update: u8) -> i32;
    }

    pub fn register() -> io::Result<()> {
        let exe = std::env::current_exe()?;
        let Some(bundle) = bundle_root(&exe) else {
            tracing::debug!(exe = %exe.display(), "not in an app bundle; brp:// links open only from a bundle");
            return Ok(());
        };
        let path = CFString::from_str(&bundle.to_string_lossy());
        let url = CFURL::with_file_system_path(
            None,
            Some(&path),
            CFURLPathStyle::CFURLPOSIXPathStyle,
            true,
        )
        .ok_or_else(|| io::Error::other(format!("{} is not a valid URL", bundle.display())))?;
        // SAFETY: a live file URL; `update` asks LaunchServices to refresh an existing record.
        let status = unsafe { LSRegisterURL(&url, 1) };
        if status != 0 {
            return Err(io::Error::other(format!("LSRegisterURL returned {status}")));
        }
        tracing::debug!(bundle = %bundle.display(), "registered with LaunchServices");
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs;
    use std::io;
    use std::path::Path;
    use std::process::Command;

    /// The id windows announce, which compositors resolve to `<id>.desktop` for their icon.
    pub const APP_ID: &str = "brp";
    /// The handler-only desktop entry, under the user's applications directory.
    pub const DESKTOP_FILE: &str = "brp.desktop";
    const ICON_FILE: &str = "brp.svg";
    const ICON_SVG: &str = include_str!("../assets/icon/openstream.svg");
    const SCHEME_MIME: &str = "x-scheme-handler/brp";

    /// The desktop entry that hands `brp://` links to `exe`. `NoDisplay` keeps it out of
    /// launchers: started from a menu without a URL, `brp join` is a usage error.
    pub fn desktop_entry(exe: &Path) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=OpenStream\nComment=Peer-to-peer screen sharing\n\
             Exec={} join %u\nIcon={APP_ID}\nTerminal=false\nNoDisplay=true\n\
             MimeType={SCHEME_MIME};\n",
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
                // The field-code rule reserves `%`, so a literal one is written `%%`.
                '%' => quoted.push('%'),
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
        write_if_changed(dir, DESKTOP_FILE, &desktop_entry(exe))
    }

    /// Writes the icon into the user's hicolor theme under `data_dir`, where the entry's `Icon`
    /// key finds it. Returns whether it wrote.
    fn install_icon(data_dir: &Path) -> io::Result<bool> {
        let dir = data_dir.join("icons/hicolor/scalable/apps");
        write_if_changed(&dir, ICON_FILE, ICON_SVG)
    }

    fn write_if_changed(dir: &Path, file: &str, contents: &str) -> io::Result<bool> {
        let path = dir.join(file);
        if fs::read_to_string(&path).is_ok_and(|current| current == contents) {
            return Ok(false);
        }
        fs::create_dir_all(dir)?;
        fs::write(&path, contents)?;
        Ok(true)
    }

    pub fn register() -> io::Result<()> {
        let exe = std::env::current_exe()?;
        let dirs = directories::BaseDirs::new()
            .ok_or_else(|| io::Error::other("no home directory for the desktop entry"))?;
        // A missing icon is cosmetic, so it must not stop the link handler registering below.
        if let Err(error) = install_icon(dirs.data_local_dir()) {
            tracing::warn!(%error, "could not install the application icon");
        }
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
        fn the_entry_names_the_icon_under_the_id_windows_announce() {
            let entry = desktop_entry(Path::new("/opt/brp/brp"));
            assert!(entry.contains(&format!("\nIcon={APP_ID}\n")), "{entry}");
            assert_eq!(DESKTOP_FILE, format!("{APP_ID}.desktop"));
            assert_eq!(ICON_FILE, format!("{APP_ID}.svg"));
        }

        #[test]
        fn the_icon_installs_once_into_the_hicolor_theme() {
            let dir = temp_dir("icon");
            let path = dir.join("icons/hicolor/scalable/apps/brp.svg");
            assert!(install_icon(&dir).unwrap());
            assert_eq!(fs::read_to_string(&path).unwrap(), ICON_SVG);
            assert!(!install_icon(&dir).unwrap());
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn the_exec_path_is_quoted_and_escaped() {
            let entry = desktop_entry(Path::new(r#"/opt/my apps/a"b`c$d\e%f/brp"#));
            let exec = entry
                .lines()
                .find(|line| line.starts_with("Exec="))
                .unwrap_or_else(|| panic!("no Exec line in {entry}"));
            assert_eq!(
                exec,
                r#"Exec="/opt/my apps/a\\"b\\`c\\$d\\\\e%%f/brp" join %u"#
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
                data: "URL:OpenStream".to_string(),
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
            let class = values
                .iter()
                .find(|v| v.subkey.is_empty() && v.name.is_none())
                .expect("class default value");
            assert_eq!(class.data, "URL:OpenStream");
            let icon = values
                .iter()
                .find(|v| v.subkey == "DefaultIcon")
                .expect("DefaultIcon value");
            assert_eq!(icon.data, r#""C:\Program Files\brp\brp.exe",0"#);
        }
    }
}
