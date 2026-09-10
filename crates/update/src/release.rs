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
}
