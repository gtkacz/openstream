//! What a release is called: the version, its tag, and the asset and directory names the release
//! workflow derives from it.

use std::fmt;
use std::str::FromStr;

use crate::error::UpdateError;
use crate::{ARCHIVE_EXTENSION, PLATFORM};

/// A plain `X.Y.Z`, the only shape the release workflow produces. Derived `Ord` compares the
/// three numbers in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl FromStr for Version {
    type Err = UpdateError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let reject = || UpdateError::Version(text.to_string());
        let mut parts = text.split('.');
        let (major, minor, patch) = (parts.next(), parts.next(), parts.next());
        if parts.next().is_some() {
            return Err(reject());
        }
        match (major, minor, patch) {
            (Some(major), Some(minor), Some(patch)) => Ok(Self(
                number(major).ok_or_else(reject)?,
                number(minor).ok_or_else(reject)?,
                number(patch).ok_or_else(reject)?,
            )),
            _ => Err(reject()),
        }
    }
}

/// Digits only: `u64::from_str` would also accept a leading `+`.
fn number(part: &str) -> Option<u64> {
    (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        .then(|| part.parse().ok())
        .flatten()
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// A published release: the version and the tag its assets live under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
}

/// The archive's top-level directory, `brp-<version>-<platform>`.
pub fn release_dir_name(version: Version) -> String {
    format!("brp-{version}-{PLATFORM}")
}

/// The archive's file name on the release page.
pub fn asset_name(version: Version) -> String {
    format!("{}{ARCHIVE_EXTENSION}", release_dir_name(version))
}

/// The release the latest-release page led to, when it is newer than `current`. `final_url` is
/// the URL after every redirect, ending in `/releases/tag/vX.Y.Z`.
pub fn newer_release(current: Version, final_url: &str) -> Result<Option<Release>, UpdateError> {
    let tag = final_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    let version: Version = tag
        .strip_prefix('v')
        .ok_or_else(|| UpdateError::Version(tag.to_string()))?
        .parse()?;
    Ok((version > current).then(|| Release {
        version,
        tag: tag.to_string(),
    }))
}

/// Length of a SHA-256 digest in hex characters.
const SHA256_HEX_LEN: usize = 64;

/// The digest listed for `asset` in a `sha256sum` output, lowercased for comparison.
pub fn checksum_for(sums: &str, asset: &str) -> Result<String, UpdateError> {
    for line in sums.lines() {
        let mut parts = line.split_whitespace();
        let (Some(digest), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if name != asset {
            continue;
        }
        let well_formed =
            digest.len() == SHA256_HEX_LEN && digest.bytes().all(|b| b.is_ascii_hexdigit());
        return if well_formed {
            Ok(digest.to_ascii_lowercase())
        } else {
            Err(UpdateError::Checksum(format!(
                "malformed digest for {asset} in {}",
                crate::CHECKSUMS_FILE
            )))
        };
    }
    Err(UpdateError::Checksum(format!(
        "{asset} is not listed in {}",
        crate::CHECKSUMS_FILE
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_three_part_versions_parse_and_print_back() {
        assert_eq!("0.4.0".parse::<Version>().unwrap(), Version(0, 4, 0));
        assert_eq!("12.3.45".parse::<Version>().unwrap(), Version(12, 3, 45));
        assert_eq!(Version(12, 3, 45).to_string(), "12.3.45");
    }

    #[test]
    fn anything_but_three_numbers_is_rejected() {
        for text in [
            "v0.4.0",
            "0.4",
            "0.4.0.1",
            "0.4.0-rc1",
            "",
            "0..0",
            "+1.2.3",
        ] {
            let error = text.parse::<Version>().unwrap_err();
            assert!(matches!(error, UpdateError::Version(_)), "{text}: {error}");
        }
    }

    #[test]
    fn versions_order_numerically_not_lexically() {
        assert!(Version(0, 4, 0) < Version(0, 10, 0));
        assert!(Version(0, 10, 0) < Version(1, 0, 0));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
    }

    #[test]
    fn asset_names_follow_the_release_workflow() {
        let version = Version(0, 5, 0);
        assert_eq!(release_dir_name(version), format!("brp-0.5.0-{PLATFORM}"));
        assert_eq!(
            asset_name(version),
            format!("brp-0.5.0-{PLATFORM}{ARCHIVE_EXTENSION}")
        );
    }

    #[test]
    fn the_tag_page_url_yields_a_release_only_when_newer() {
        let url = "https://github.com/gtkacz/openstream/releases/tag/v0.5.0";
        assert_eq!(
            newer_release(Version(0, 4, 0), url).unwrap(),
            Some(Release {
                version: Version(0, 5, 0),
                tag: "v0.5.0".into(),
            })
        );
        assert_eq!(newer_release(Version(0, 5, 0), url).unwrap(), None);
        assert_eq!(newer_release(Version(0, 6, 0), url).unwrap(), None);
        // A trailing slash is how some redirects arrive.
        assert!(
            newer_release(Version(0, 4, 0), &format!("{url}/"))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn a_url_that_does_not_end_in_a_tag_is_a_version_error() {
        for url in [
            "https://github.com/gtkacz/openstream/releases",
            "https://github.com/gtkacz/openstream/releases/tag/0.5.0",
            "https://github.com/gtkacz/openstream/releases/tag/nightly",
            "",
        ] {
            let error = newer_release(Version(0, 4, 0), url).unwrap_err();
            assert!(matches!(error, UpdateError::Version(_)), "{url}: {error}");
        }
    }

    const SUMS: &str = "\
f6c4cab76618ef2b810f99c352684851226d7b0abd43ee9d4e62d934063a33a1  brp-0.4.0-linux-x86_64.tar.gz
E7DA9DC799608DB8A324DBA72CFC37B8B76B375B927A7AEF9A90CE758915A7F7  brp-0.4.0-windows-x86_64.zip
";

    #[test]
    fn the_digest_of_the_named_asset_is_found_and_lowercased() {
        assert_eq!(
            checksum_for(SUMS, "brp-0.4.0-linux-x86_64.tar.gz").unwrap(),
            "f6c4cab76618ef2b810f99c352684851226d7b0abd43ee9d4e62d934063a33a1"
        );
        assert_eq!(
            checksum_for(SUMS, "brp-0.4.0-windows-x86_64.zip").unwrap(),
            "e7da9dc799608db8a324dba72cfc37b8b76b375b927a7aef9a90ce758915a7f7"
        );
    }

    #[test]
    fn a_missing_asset_or_a_malformed_digest_is_a_checksum_error() {
        let missing = checksum_for(SUMS, "brp-0.4.0-macos.tar.gz").unwrap_err();
        assert!(matches!(missing, UpdateError::Checksum(_)), "{missing}");
        let short = checksum_for("abc123  brp.tar.gz\n", "brp.tar.gz").unwrap_err();
        assert!(matches!(short, UpdateError::Checksum(_)), "{short}");
        let not_hex =
            checksum_for(&format!("{}  brp.tar.gz\n", "g".repeat(64)), "brp.tar.gz").unwrap_err();
        assert!(matches!(not_hex, UpdateError::Checksum(_)), "{not_hex}");
    }
}
