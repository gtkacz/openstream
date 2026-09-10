//! Unpacking a release archive into the staging directory. Both formats are compiled on every
//! platform so the Windows extractor is exercised by the Linux suite.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Component, Path};

use flate2::read::GzDecoder;

use crate::error::UpdateError;

/// Unpacks the platform's archive format. Returns the file names written into `into`.
pub fn extract(archive: &Path, top_level: &str, into: &Path) -> Result<Vec<OsString>, UpdateError> {
    if cfg!(windows) {
        extract_zip(archive, top_level, into)
    } else {
        extract_tar_gz(archive, top_level, into)
    }
}

/// The Linux release: one top-level directory of regular files whose Unix modes matter, since
/// `brp` and the shared libraries must stay executable.
pub fn extract_tar_gz(
    archive: &Path,
    top_level: &str,
    into: &Path,
) -> Result<Vec<OsString>, UpdateError> {
    let file = File::open(archive)?;
    let mut tar = tar::Archive::new(GzDecoder::new(BufReader::new(file)));
    let mut files = Vec::new();
    for entry in tar.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        let path = entry.path()?.into_owned();
        if !kind.is_file() {
            return Err(UpdateError::Archive(format!(
                "{} is not a regular file",
                path.display()
            )));
        }
        let name = member_name(&path, top_level)?;
        // `unpack` applies the header's mode on Unix and refuses to follow links; the type and
        // path checks above already rejected anything that is not a plain file.
        entry.unpack(into.join(&name))?;
        files.push(name);
    }
    Ok(files)
}

/// The Windows release. Entry names are normalised to forward slashes first because
/// `Compress-Archive` has written backslashes in some PowerShell versions.
pub fn extract_zip(
    archive: &Path,
    top_level: &str,
    into: &Path,
) -> Result<Vec<OsString>, UpdateError> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?).map_err(archive_error)?;
    let mut files = Vec::new();
    for index in 0..zip.len() {
        let mut member = zip.by_index(index).map_err(archive_error)?;
        if member.is_dir() {
            continue;
        }
        if member.is_symlink() {
            return Err(UpdateError::Archive(format!("{} is a link", member.name())));
        }
        let normalised = member.name().replace('\\', "/");
        let name = member_name(Path::new(&normalised), top_level)?;
        let mut out = File::create(into.join(&name))?;
        io::copy(&mut member, &mut out)?;
        files.push(name);
    }
    Ok(files)
}

fn archive_error(error: zip::result::ZipError) -> UpdateError {
    UpdateError::Archive(error.to_string())
}

/// The file name of an entry that is exactly `<top_level>/<name>`. Anything else, including
/// `..`, an absolute path, a nested directory, or another top level, is rejected so a crafted
/// archive cannot write outside the staging directory.
pub fn member_name(path: &Path, top_level: &str) -> Result<OsString, UpdateError> {
    let mut components = path.components();
    match (components.next(), components.next(), components.next()) {
        (Some(Component::Normal(top)), Some(Component::Normal(name)), None)
            if top == top_level && !name.is_empty() =>
        {
            Ok(name.to_os_string())
        }
        _ => Err(UpdateError::Archive(format!(
            "unexpected entry {}",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    const TOP: &str = "brp-0.6.0-test";

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brp-archive-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A tar header whose name is written byte for byte, because `Header::set_path` refuses `..`
    /// and absolute paths and the tests need exactly those.
    fn raw_header(name: &str, size: u64, mode: u32) -> tar::Header {
        let mut header = tar::Header::new_gnu();
        header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
        header.set_size(size);
        header.set_mode(mode);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        header
    }

    fn write_tar_gz(path: &Path, entries: &[(&str, &[u8], u32)], with_dir: bool) {
        let file = File::create(path).unwrap();
        let mut builder = tar::Builder::new(GzEncoder::new(file, Compression::fast()));
        if with_dir {
            let mut header = tar::Header::new_gnu();
            header.set_path(format!("{TOP}/")).unwrap();
            header.set_size(0);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::Directory);
            header.set_cksum();
            builder.append(&header, io::empty()).unwrap();
        }
        for (name, data, mode) in entries {
            builder
                .append(&raw_header(name, data.len() as u64, *mode), *data)
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn member_names_are_one_plain_name_under_the_top_level_directory() {
        assert_eq!(
            member_name(Path::new("top/brp"), "top").unwrap(),
            OsString::from("brp")
        );
        for bad in [
            "other/brp",
            "top/sub/brp",
            "top/../escape",
            "/top/brp",
            "top",
            "top/",
            "./top/brp",
        ] {
            let error = member_name(Path::new(bad), "top").unwrap_err();
            assert!(matches!(error, UpdateError::Archive(_)), "{bad}: {error}");
        }
    }

    #[test]
    fn a_tarball_extracts_its_files_with_their_modes_and_skips_the_directory_entry() {
        let dir = temp_dir("tar_ok");
        let archive = dir.join("release.tar.gz");
        write_tar_gz(
            &archive,
            &[
                (&format!("{TOP}/brp"), b"binary", 0o755),
                (&format!("{TOP}/LICENSE"), b"MIT", 0o644),
            ],
            true,
        );
        let out = dir.join("out");
        fs::create_dir(&out).unwrap();
        let mut files = extract_tar_gz(&archive, TOP, &out).unwrap();
        files.sort();
        assert_eq!(files, [OsString::from("LICENSE"), OsString::from("brp")]);
        assert_eq!(fs::read(out.join("brp")).unwrap(), b"binary");
        assert_eq!(fs::read(out.join("LICENSE")).unwrap(), b"MIT");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(out.join("brp")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "{mode:o}");
        }
    }

    #[test]
    fn a_tarball_with_an_escaping_nested_or_foreign_entry_is_rejected_before_writing() {
        for (case, name) in [
            ("parent", format!("{TOP}/../escape")),
            ("absolute", "/tmp/escape".to_string()),
            ("nested", format!("{TOP}/sub/brp")),
            ("wrong_top", "brp-9.9.9-other/brp".to_string()),
        ] {
            let dir = temp_dir(&format!("tar_bad_{case}"));
            let archive = dir.join("release.tar.gz");
            write_tar_gz(
                &archive,
                &[
                    (&format!("{TOP}/brp"), b"binary", 0o755),
                    (&name, b"x", 0o644),
                ],
                false,
            );
            let out = dir.join("out");
            fs::create_dir(&out).unwrap();
            let error = extract_tar_gz(&archive, TOP, &out).unwrap_err();
            assert!(matches!(error, UpdateError::Archive(_)), "{case}: {error}");
            // The good entry before the bad one may have been written; the bad one never is.
            assert!(!dir.join("escape").exists(), "{case}");
            assert!(!out.join("sub").exists(), "{case}");
            assert!(!Path::new("/tmp/escape").exists(), "{case}");
        }
    }

    #[test]
    fn a_zip_extracts_forward_and_backslash_entries_alike_and_skips_directories() {
        let dir = temp_dir("zip_ok");
        let archive = dir.join("release.zip");
        write_zip(
            &archive,
            &[
                (&format!("{TOP}/"), b""),
                (&format!("{TOP}/brp.exe"), b"binary"),
                (&format!("{TOP}\\avcodec-62.dll"), b"codec"),
            ],
        );
        let out = dir.join("out");
        fs::create_dir(&out).unwrap();
        let mut files = extract_zip(&archive, TOP, &out).unwrap();
        files.sort();
        assert_eq!(
            files,
            [OsString::from("avcodec-62.dll"), OsString::from("brp.exe")]
        );
        assert_eq!(listing(&out), ["avcodec-62.dll", "brp.exe"]);
        assert_eq!(fs::read(out.join("brp.exe")).unwrap(), b"binary");
    }

    #[test]
    fn a_zip_with_an_escaping_or_nested_entry_is_rejected() {
        for (case, name) in [
            ("parent", format!("{TOP}/../escape")),
            ("nested", format!("{TOP}/sub/brp.exe")),
            ("wrong_top", "other/brp.exe".to_string()),
        ] {
            let dir = temp_dir(&format!("zip_bad_{case}"));
            let archive = dir.join("release.zip");
            write_zip(&archive, &[(&name, b"x")]);
            let out = dir.join("out");
            fs::create_dir(&out).unwrap();
            let error = extract_zip(&archive, TOP, &out).unwrap_err();
            assert!(matches!(error, UpdateError::Archive(_)), "{case}: {error}");
            assert!(listing(&out).is_empty(), "{case}");
        }
    }

    #[test]
    fn extract_uses_the_platform_format() {
        let dir = temp_dir("platform");
        let archive = dir.join(format!("release{}", crate::ARCHIVE_EXTENSION));
        let entry = format!("{TOP}/file");
        if cfg!(windows) {
            write_zip(&archive, &[(&entry, b"x")]);
        } else {
            write_tar_gz(&archive, &[(&entry, b"x", 0o644)], false);
        }
        let out = dir.join("out");
        fs::create_dir(&out).unwrap();
        assert_eq!(
            extract(&archive, TOP, &out).unwrap(),
            [OsString::from("file")]
        );
    }
}
