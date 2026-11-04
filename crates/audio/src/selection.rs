//! What a capture session carries: the identity a selection is stored under, the applications the
//! platform reports as audible, and the choice itself. Platform-neutral and pure, so the picker's
//! logic and both backends' predicates are testable on one runner.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, PoisonError, RwLock};

/// The identity a selection is stored under: an executable basename, normalised lowercase on
/// Windows, or a bundle identifier, lowercase, on macOS. Never a pid — pids do not survive a restart, and one application often owns several
/// audio streams and several processes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppKey(String);

impl AppKey {
    /// From a basename or a full path, whichever the platform reported or the settings file holds.
    pub fn new(name: &str) -> Self {
        Self(normalised(name))
    }

    /// From a macOS bundle identifier, under the macOS rule whatever platform compiles it, so
    /// the macOS backend's logic is tested on every runner. On macOS it equals [`AppKey::new`].
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub(crate) fn bundle(id: &str) -> Self {
        Self(macos_normalised(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Windows accepts either separator and ignores case in file names; Linux does neither, and macOS
/// ignores case in bundle identifiers, so the same settings file behaves like the platform that
/// reads it.
#[cfg(windows)]
fn normalised(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_lowercase()
}

#[cfg(target_os = "macos")]
fn normalised(name: &str) -> String {
    macos_normalised(name)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn normalised(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_string()
}

/// macOS keys are bundle identifiers, which the system compares without case; a path, as a
/// hand-edited settings file may hold, keeps its last component. Compiled everywhere so the rule
/// is tested on every runner.
fn macos_normalised(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_lowercase()
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

/// How loud each application reaches the room, in percent; an application it does not name is at
/// full volume. Shared with running sessions, so a change takes effect without swapping the session
/// and gapping the room as a selection change does. The capture callback only holds the lock long
/// enough to read a number, and already allocates per chunk, so a lock is not what threatens it.
#[derive(Debug, Clone, Default)]
pub struct SendVolumes(Arc<RwLock<BTreeMap<AppKey, u8>>>);

impl SendVolumes {
    pub fn set(&self, volumes: BTreeMap<AppKey, u8>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = volumes;
    }

    /// The multiplier for audio owned by this identity. A stream with no identity cannot have been
    /// given a volume, so it stays at full.
    pub fn gain(&self, key: Option<&AppKey>) -> f32 {
        let volumes = self.0.read().unwrap_or_else(PoisonError::into_inner);
        key.and_then(|key| volumes.get(key))
            .map_or(1.0, |percent| f32::from((*percent).min(100)) / 100.0)
    }
}

/// When a backend can tell one application's audio from another's before mixing it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SendVolumeSupport {
    /// The platform hands over one finished mix.
    #[default]
    Never,
    /// Only under [`AudioSelection::Only`]; [`AudioSelection::All`] is one finished mix.
    OnlySelected,
    Always,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_application_gets_its_volume_and_anything_else_stays_at_full() {
        let volumes = SendVolumes::default();
        volumes.set(BTreeMap::from([(AppKey::new("game"), 25)]));
        assert_eq!(volumes.gain(Some(&AppKey::new("game"))), 0.25);
        assert_eq!(volumes.gain(Some(&AppKey::new("firefox"))), 1.0);
        assert_eq!(volumes.gain(None), 1.0);
    }

    #[test]
    fn a_volume_above_full_from_a_hand_edited_file_is_read_as_full() {
        let volumes = SendVolumes::default();
        volumes.set(BTreeMap::from([(AppKey::new("game"), 250)]));
        assert_eq!(volumes.gain(Some(&AppKey::new("game"))), 1.0);
    }

    #[test]
    fn a_session_holding_a_clone_sees_a_later_change() {
        let volumes = SendVolumes::default();
        let session = volumes.clone();
        volumes.set(BTreeMap::from([(AppKey::new("game"), 0)]));
        assert_eq!(session.gain(Some(&AppKey::new("game"))), 0.0);
    }

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

    #[test]
    fn macos_keys_are_bundle_identifiers_compared_without_case() {
        assert_eq!(
            macos_normalised("com.Apple.Safari"),
            macos_normalised("com.apple.safari")
        );
        assert_eq!(macos_normalised("com.apple.Safari"), "com.apple.safari");
        assert_eq!(
            macos_normalised("/Applications/Spotify.app/Contents/MacOS/Spotify"),
            "spotify"
        );
        assert_eq!(macos_normalised(""), "");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_builds_use_the_macos_rule() {
        assert_eq!(
            AppKey::new("COM.SPOTIFY.client"),
            AppKey::new("com.spotify.client")
        );
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
