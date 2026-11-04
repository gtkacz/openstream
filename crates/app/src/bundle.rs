//! The macOS application bundle: the identifiers its `Info.plist` declares, and finding the bundle
//! the running binary was started from. Pure, so every runner checks it against the template the
//! staging script fills in.

use std::ffi::OsString;
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

/// One command of a bundle refresh. Only system tools every macOS ships are used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub program: &'static str,
    pub args: Vec<OsString>,
}

const PLUTIL: &str = "/usr/bin/plutil";
const CODESIGN: &str = "/usr/bin/codesign";

/// What makes a bundle whole again after an update swapped the files in `Contents/MacOS`
/// (`files`): the new version in its Info.plist, which the update archive does not carry, then an
/// ad-hoc signature for every file but the main executable, then one for the bundle, which signs
/// the executable over the Info.plist and everything beside it, and a check of the result.
/// Without it the bundle's seal no longer matches what is inside.
pub fn refresh_steps(bundle: &Path, files: &[PathBuf], version: &str) -> Vec<Step> {
    let plist = bundle.join("Contents/Info.plist");
    let executable = bundle.join("Contents/MacOS").join(EXECUTABLE);
    let step = |program, args: Vec<OsString>| Step { program, args };
    let mut steps: Vec<Step> = ["CFBundleShortVersionString", "CFBundleVersion"]
        .into_iter()
        .map(|key| {
            step(
                PLUTIL,
                vec![
                    "-replace".into(),
                    key.into(),
                    "-string".into(),
                    version.into(),
                    plist.clone().into(),
                ],
            )
        })
        .collect();
    steps.extend(
        files
            .iter()
            .filter(|file| **file != executable)
            .map(|file| {
                step(
                    CODESIGN,
                    vec!["--force".into(), "--sign".into(), "-".into(), file.into()],
                )
            }),
    );
    steps.push(step(
        CODESIGN,
        vec!["--force".into(), "--sign".into(), "-".into(), bundle.into()],
    ));
    steps.push(step(
        CODESIGN,
        vec!["--verify".into(), "--strict".into(), bundle.into()],
    ));
    steps
}

/// Refreshes the bundle `install` lives in after an update to `version`: the previous version's
/// `.old` files go first, since nothing on macOS still needs them and the seal would cover them,
/// then [`refresh_steps`] runs. Does nothing for an install outside a bundle.
#[cfg(target_os = "macos")]
pub fn refresh_after_update(install: &brp_update::Install, version: &str) -> std::io::Result<()> {
    let Some(bundle) = bundle_root(&install.exe) else {
        return Ok(());
    };
    brp_update::cleanup_stale(install);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&install.dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    for step in refresh_steps(&bundle, &files, version) {
        let output = std::process::Command::new(step.program)
            .args(&step.args)
            .output()?;
        if !output.status.success() {
            return Err(std::io::Error::other(format!(
                "{} {:?} failed: {}",
                step.program,
                step.args,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
    }
    tracing::info!(bundle = %bundle.display(), version, "bundle refreshed after the update");
    Ok(())
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

    #[cfg(unix)]
    fn line(step: &Step) -> String {
        std::iter::once(step.program.to_string())
            .chain(step.args.iter().map(|a| a.to_string_lossy().into_owned()))
            .collect::<Vec<_>>()
            .join(" ")
    }

    // Paths joined on Windows would carry backslashes, and only macOS ever runs these steps.
    #[cfg(unix)]
    #[test]
    fn an_updated_bundle_takes_the_new_version_then_is_signed_inside_out_and_verified() {
        let bundle = Path::new("/Applications/OpenStream.app");
        let files = [
            bundle.join("Contents/MacOS/libavcodec.62.dylib"),
            bundle.join("Contents/MacOS/brp"),
            bundle.join("Contents/MacOS/FFMPEG-LICENSE.txt"),
        ];
        let steps: Vec<String> = refresh_steps(bundle, &files, "1.3.0")
            .iter()
            .map(line)
            .collect();
        let plist = "/Applications/OpenStream.app/Contents/Info.plist";
        let macos = "/Applications/OpenStream.app/Contents/MacOS";
        assert_eq!(
            steps,
            [
                format!(
                    "/usr/bin/plutil -replace CFBundleShortVersionString -string 1.3.0 {plist}"
                ),
                format!("/usr/bin/plutil -replace CFBundleVersion -string 1.3.0 {plist}"),
                format!("/usr/bin/codesign --force --sign - {macos}/libavcodec.62.dylib"),
                format!("/usr/bin/codesign --force --sign - {macos}/FFMPEG-LICENSE.txt"),
                "/usr/bin/codesign --force --sign - /Applications/OpenStream.app".to_string(),
                "/usr/bin/codesign --verify --strict /Applications/OpenStream.app".to_string(),
            ],
            "the main executable is signed by the bundle step, after everything it seals"
        );
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
