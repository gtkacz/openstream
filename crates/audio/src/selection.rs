//! What a capture session carries: the identity a selection is stored under, the applications the
//! platform reports as audible, and the choice itself. Platform-neutral and pure, so the picker's
//! logic and both backends' predicates are testable on one runner.

use std::collections::BTreeSet;

/// The identity a selection is stored under: an executable basename, normalised lowercase on
/// Windows. Never a pid — pids do not survive a restart, and one application often owns several
/// audio streams and several processes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppKey(String);

impl AppKey {
    /// From a basename or a full path, whichever the platform reported or the settings file holds.
    pub fn new(name: &str) -> Self {
        Self(normalised(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Windows accepts either separator and ignores case in file names; Linux does neither, so the
/// same settings file behaves like the platform that reads it.
#[cfg(windows)]
fn normalised(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_lowercase()
}

#[cfg(not(windows))]
fn normalised(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_string()
}

/// One application the platform reports as producing audio right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSource {
    pub key: AppKey,
    /// What the user sees: the application's own name, or its identity when it has none.
    pub label: String,
}

/// What a capture session carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AudioSelection {
    /// Everything the machine plays except brp itself — phase 4's behaviour, and the default.
    #[default]
    All,
    /// Only these. An empty set is silence, which is what the mode says.
    Only(BTreeSet<AppKey>),
}

impl AudioSelection {
    /// Whether audio owned by this identity is captured. A stream whose owner could not be
    /// identified has no key: it is captured under `All`, where the key is not load-bearing, and
    /// left out under `Only`, where sharing an application the user did not name is the failure
    /// this feature exists to prevent.
    pub fn admits(&self, key: Option<&AppKey>) -> bool {
        match self {
            Self::All => true,
            Self::Only(keys) => key.is_some_and(|key| keys.contains(key)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_the_basename_of_whatever_it_is_given() {
        assert_eq!(AppKey::new("firefox").as_str(), "firefox");
        assert_eq!(AppKey::new("/usr/bin/firefox").as_str(), "firefox");
        assert_eq!(AppKey::new("").as_str(), "");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_matches_a_basename_exactly() {
        assert_eq!(AppKey::new("Firefox").as_str(), "Firefox");
        assert_ne!(AppKey::new("Firefox"), AppKey::new("firefox"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_matches_case_insensitively_and_takes_either_separator() {
        assert_eq!(AppKey::new("Game.exe"), AppKey::new("GAME.EXE"));
        assert_eq!(AppKey::new(r"C:\Games\Game.exe"), AppKey::new("game.exe"));
        assert_eq!(AppKey::new("C:/Games/Game.exe"), AppKey::new("game.exe"));
    }

    #[test]
    fn all_is_the_default_and_admits_everything_including_an_unknown_identity() {
        let all = AudioSelection::default();
        assert_eq!(all, AudioSelection::All);
        assert!(all.admits(Some(&AppKey::new("firefox"))));
        assert!(all.admits(None), "under All a key is not load-bearing");
    }

    #[test]
    fn only_admits_its_members_and_never_an_unknown_identity() {
        let only = AudioSelection::Only(BTreeSet::from([AppKey::new("firefox")]));
        assert!(only.admits(Some(&AppKey::new("firefox"))));
        assert!(!only.admits(Some(&AppKey::new("spotify"))));
        assert!(
            !only.admits(None),
            "an unresolved identity fails closed under Only"
        );
    }

    #[test]
    fn an_empty_only_set_is_silence() {
        let nothing = AudioSelection::Only(BTreeSet::new());
        assert!(!nothing.admits(Some(&AppKey::new("firefox"))));
        assert!(!nothing.admits(None));
    }
}
