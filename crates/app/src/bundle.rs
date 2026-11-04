//! The macOS application bundle: the identifiers its `Info.plist` declares, and finding the bundle
//! the running binary was started from. Pure, so every runner checks it against the template the
//! staging script fills in.

use std::path::{Path, PathBuf};

/// The bundle identifier LaunchServices knows the app by; audio capture excludes it too.
pub const BUNDLE_ID: &str = brp_proto::constants::MACOS_BUNDLE_ID;
/// The binary inside `Contents/MacOS`.
pub const EXECUTABLE: &str = brp_proto::constants::EXECUTABLE_NAME;
/// The scheme the bundle claims for join links.
pub const URL_SCHEME: &str = "brp";
/// The oldest macOS the bundle runs on: ScreenCaptureKit's content sizing needs 14.
pub const MINIMUM_MACOS: &str = "14.0";

/// The `.app` directory `exe` sits in, when it is the bundle's own executable at
/// `<name>.app/Contents/MacOS/<exe>`. A binary anywhere else (an extracted tarball, a `cargo
/// build`) has no bundle, and so no Info.plist to register.
pub fn bundle_root(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let root = contents.parent()?;
    let is_app = root
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"));
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && is_app)
        .then(|| root.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO_PLIST: &str = include_str!("../../../ci/macos/Info.plist");

    /// The `<string>` that follows `<key>name</key>` in the template.
    fn value(key: &str) -> &'static str {
        let marker = format!("<key>{key}</key>");
        let rest = &INFO_PLIST[INFO_PLIST
            .find(&marker)
            .unwrap_or_else(|| panic!("{key} is not in Info.plist"))
            + marker.len()..];
        let start = rest.find("<string>").unwrap() + "<string>".len();
        &rest[start..start + rest[start..].find("</string>").unwrap()]
    }

    #[test]
    fn the_bundle_declares_what_the_app_relies_on() {
        assert_eq!(value("CFBundleIdentifier"), BUNDLE_ID);
        assert_eq!(value("CFBundleExecutable"), EXECUTABLE);
        assert_eq!(value("LSMinimumSystemVersion"), MINIMUM_MACOS);
        assert_eq!(value("CFBundleShortVersionString"), "@VERSION@");
        let schemes = &INFO_PLIST[INFO_PLIST.find("<key>CFBundleURLSchemes</key>").unwrap()..];
        assert!(
            schemes.contains(&format!("<string>{URL_SCHEME}</string>")),
            "the bundle must claim the scheme the join links use"
        );
        assert!(crate::link::SCHEME_JOIN_PREFIX.starts_with(&format!("{URL_SCHEME}://")));
    }

    #[test]
    fn a_binary_inside_a_bundle_finds_the_bundle() {
        assert_eq!(
            bundle_root(Path::new("/Applications/OpenStream.app/Contents/MacOS/brp")),
            Some(PathBuf::from("/Applications/OpenStream.app"))
        );
        assert_eq!(
            bundle_root(Path::new(
                "/Users/a/My Apps/Open Stream.app/Contents/MacOS/brp"
            )),
            Some(PathBuf::from("/Users/a/My Apps/Open Stream.app"))
        );
    }

    #[test]
    fn a_binary_outside_a_bundle_has_none() {
        for exe in [
            "/opt/brp-1.2.0-macos-aarch64/brp",
            "/Applications/OpenStream.app/brp",
            "/Applications/OpenStream/Contents/MacOS/brp",
            "/Applications/OpenStream.app/Contents/Resources/brp",
            "brp",
            "/",
        ] {
            assert_eq!(bundle_root(Path::new(exe)), None, "{exe}");
        }
    }
}
