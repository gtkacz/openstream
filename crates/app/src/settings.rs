//! What persists across launches besides the identity key: one TOML file in the platform config
//! directory, read once at startup and written whole after a room opens or the dialog saves.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use brp_audio::{AppKey, AudioSelection};
use brp_net::RelaySetting;
use brp_voiceproc::{ProcessingConfig, Suppression};
use directories::ProjectDirs;
use iroh::RelayUrl;
use serde::{Deserialize, Serialize};

use crate::cli::DEFAULT_FPS;
use crate::error::AppError;
use crate::identity::write_private;

/// Beside `identity.key` in the platform config directory.
pub const SETTINGS_FILE: &str = "settings.toml";
/// Enough to cover a week of rooms for a small group without scrolling the start screen.
pub const RECENT_ROOMS_MAX: usize = 8;

/// Everything the file holds. Unknown keys are ignored on load so an older binary reads a newer
/// file; missing keys take their defaults so a newer binary reads an older one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub nickname: Option<String>,
    pub fps: u32,
    pub relay: RelayChoice,
    pub audio: AudioSettings,
    pub recent_rooms: Vec<RecentRoom>,
    /// Ask GitHub for the latest release at launch. On by default and documented as a network
    /// request, which is why it can be turned off.
    pub check_updates: bool,
    /// A `brp://` link joins its room at once instead of asking. Off by default because any web page
    /// can fire one, and joining shows the room this person's name and IP address.
    pub auto_join_links: bool,
    /// Join voice as soon as a room opens. Off by default because it opens the microphone in
    /// every room this person enters.
    pub auto_join_voice: bool,
    /// The colour of the tube. Kept in the file but not applied while choosing one is disabled.
    pub phosphor: crate::ui::theme::Phosphor,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            nickname: None,
            fps: DEFAULT_FPS,
            relay: RelayChoice::Default,
            audio: AudioSettings::default(),
            recent_rooms: Vec::new(),
            check_updates: true,
            auto_join_links: false,
            auto_join_voice: false,
            phosphor: crate::ui::theme::Phosphor::default(),
        }
    }
}

/// Serialised as `[relay] mode = "default" | "custom" | "disabled"`, with `url` for custom.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "url", rename_all = "lowercase")]
pub enum RelayChoice {
    #[default]
    Default,
    Custom(String),
    Disabled,
}

impl RelayChoice {
    /// The transport setting, or the parse error of a custom URL.
    pub fn to_relay_setting(&self) -> Result<RelaySetting, AppError> {
        Ok(match self {
            RelayChoice::Default => RelaySetting::Default,
            RelayChoice::Disabled => RelaySetting::Disabled,
            RelayChoice::Custom(url) => {
                RelaySetting::Custom(url.parse::<RelayUrl>().map_err(|e| {
                    AppError::Settings(format!(
                        "Enter a relay URL like https://relay.example.com/ ({url:?}: {e})"
                    ))
                })?)
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// A cpal device id in its `Display` form; `None` is the system default.
    pub output_device: Option<String>,
    /// The microphone, in the same form as `output_device`.
    pub input_device: Option<String>,
    /// Used for voice only while `separate_voice_output` is set.
    pub voice_output_device: Option<String>,
    pub separate_voice_output: bool,
    pub applications: AudioApplications,
    pub echo_cancellation: bool,
    pub noise_suppression: NoiseSuppression,
    pub automatic_gain: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            output_device: None,
            input_device: None,
            voice_output_device: None,
            separate_voice_output: false,
            applications: AudioApplications::default(),
            echo_cancellation: true,
            noise_suppression: NoiseSuppression::default(),
            automatic_gain: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoiseSuppression {
    Off,
    #[default]
    Light,
}

impl AudioSettings {
    /// What the microphone goes through before voice is encoded; mirrors
    /// [`AudioApplications::to_selection`].
    pub fn voice_processing(&self) -> ProcessingConfig {
        ProcessingConfig {
            echo_cancellation: self.echo_cancellation,
            noise_suppression: match self.noise_suppression {
                NoiseSuppression::Off => Suppression::Off,
                NoiseSuppression::Light => Suppression::Light,
            },
            automatic_gain: self.automatic_gain,
        }
    }

    /// The device voice plays through: the audio output unless voice was given its own.
    pub fn voice_output(&self) -> Option<String> {
        if self.separate_voice_output {
            self.voice_output_device.clone()
        } else {
            self.output_device.clone()
        }
    }
}

/// Serialised as `[audio.applications] mode = "all" | "only"` with `names` and, under
/// `[audio.applications.volumes]`, the percent of any application not at full volume. `mode` and
/// `names` are always written: flipping to every application for a film and back must not lose the set. This
/// deliberately differs from [`RelayChoice`]'s adjacently-tagged shape, whose variants carry
/// genuinely different data.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioApplications {
    pub mode: AudioMode,
    pub names: Vec<String>,
    pub volumes: BTreeMap<String, u8>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioMode {
    #[default]
    All,
    Only,
}

impl AudioApplications {
    /// How loud each application reaches the room. Names go through [`AppKey`] as in
    /// [`Self::to_selection`].
    pub fn to_volumes(&self) -> BTreeMap<AppKey, u8> {
        self.volumes
            .iter()
            .map(|(name, percent)| (AppKey::new(name), *percent))
            .collect()
    }

    /// What the room captures with. Names go through [`AppKey`], so a hand-edited file with odd
    /// casing still matches on Windows; mirrors [`RelayChoice::to_relay_setting`].
    pub fn to_selection(&self) -> AudioSelection {
        match self.mode {
            AudioMode::All => AudioSelection::All,
            AudioMode::Only => {
                AudioSelection::Only(self.names.iter().map(|name| AppKey::new(name)).collect())
            }
        }
    }
}

/// A room the user created or joined: the ticket that got them in, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentRoom {
    pub ticket: String,
    pub last_joined_unix: u64,
}

impl Settings {
    pub fn path() -> Result<PathBuf, AppError> {
        let dirs = ProjectDirs::from("", "", "brp").ok_or_else(|| {
            AppError::Settings(
                "No home directory to store settings in. Run OpenStream as a user with a \
                 home directory."
                    .into(),
            )
        })?;
        Ok(dirs.config_dir().join(SETTINGS_FILE))
    }

    /// A missing file is the defaults. Any other failure, including a value `validate` rejects,
    /// is an error and the file is left as is.
    pub fn load(path: &Path) -> Result<Self, AppError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(AppError::Settings(format!("{}: {e}", path.display()))),
        };
        let settings: Self = toml::from_str(&text)
            .map_err(|e| AppError::Settings(format!("{}: {e}", path.display())))?;
        settings
            .validate()
            .map_err(|msg| AppError::Settings(format!("{}: {msg}", path.display())))?;
        Ok(settings)
    }

    /// Writes the whole file through a temporary sibling and a rename, so a crash mid-write
    /// leaves the previous file intact.
    pub fn save(&self, path: &Path) -> Result<(), AppError> {
        let text = toml::to_string_pretty(self).map_err(|e| AppError::Settings(e.to_string()))?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)
                .map_err(|e| AppError::Settings(format!("{}: {e}", path.display())))?;
        }
        // Unique per process: two instances saving at once must not share a temp sibling, or one
        // instance's rename could pick up the other's still-being-written file.
        let tmp = path.with_extension(format!("toml.{}.tmp", std::process::id()));
        // Owner-only, because the file holds tickets and a ticket is all it takes to join. A leftover
        // from a crashed run with this process id is cleared first, or the write would refuse it.
        let _ = fs::remove_file(&tmp);
        write_private(&tmp, text.as_bytes())
            .map_err(|e| AppError::Settings(format!("{}: {e}", path.display())))?;
        fs::rename(&tmp, path)
            .map_err(|e| AppError::Settings(format!("{}: {e}", path.display())))?;
        Ok(())
    }

    /// Puts `ticket` first with `now_unix`, dropping an older entry for the same ticket and
    /// anything beyond [`RECENT_ROOMS_MAX`].
    pub fn remember_room(&mut self, ticket: &str, now_unix: u64) {
        self.recent_rooms.retain(|room| room.ticket != ticket);
        self.recent_rooms.insert(
            0,
            RecentRoom {
                ticket: ticket.to_string(),
                last_joined_unix: now_unix,
            },
        );
        self.recent_rooms.truncate(RECENT_ROOMS_MAX);
    }

    /// Swaps a remembered room's ticket for a newer way into the same room, and puts it first.
    pub fn replace_room(&mut self, old_ticket: &str, ticket: &str, now_unix: u64) {
        self.recent_rooms.retain(|room| room.ticket != old_ticket);
        self.remember_room(ticket, now_unix);
    }

    /// What Save refuses: a zero frame rate, or a custom relay URL that does not parse.
    pub fn validate(&self) -> Result<(), String> {
        if self.fps == 0 {
            return Err("Set the frame rate ceiling to at least 1 fps".into());
        }
        self.relay
            .to_relay_setting()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// The settings together with where they live and whether loading them failed; the app owns one.
#[derive(Debug)]
pub struct SettingsStore {
    pub settings: Settings,
    pub path: PathBuf,
    /// Set when the file exists but could not be read or parsed. Shown on the start screen; the
    /// defaults are in use and nothing is written until the user saves.
    pub load_error: Option<String>,
}

impl SettingsStore {
    pub fn load() -> Result<Self, AppError> {
        Ok(Self::load_at(Settings::path()?))
    }

    pub fn load_at(path: PathBuf) -> Self {
        let (settings, load_error) = match Settings::load(&path) {
            Ok(settings) => (settings, None),
            // `Settings` variant already names the path; other variants need their own message.
            Err(AppError::Settings(message)) => (Settings::default(), Some(message)),
            Err(error) => (Settings::default(), Some(error.to_string())),
        };
        Self {
            settings,
            path,
            load_error,
        }
    }

    /// Saves and, on success, clears a load error: the file is now what we wrote.
    pub fn save(&mut self) -> Result<(), AppError> {
        self.settings.save(&self.path)?;
        self.load_error = None;
        Ok(())
    }

    /// Saves unless the file failed to load. After a load failure the defaults are in use, and
    /// writing them would replace the user's file with defaults; only an explicit Save from the
    /// dialog may do that. Returns whether a write happened.
    pub fn save_unless_load_failed(&mut self) -> Result<bool, AppError> {
        if self.load_error.is_some() {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }
}

/// Seconds since the Unix epoch, or zero if the clock is before it.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The nickname rule shared by the dialog and the start screen: surrounding whitespace is
/// dropped and an empty result means "no saved nickname".
pub fn normalised_nickname(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn temp_path(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brp-settings-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join(SETTINGS_FILE)
    }

    fn full() -> Settings {
        Settings {
            phosphor: crate::ui::theme::Phosphor::Green,
            nickname: Some("alice".into()),
            fps: 30,
            relay: RelayChoice::Custom("https://relay.example.com/".into()),
            audio: AudioSettings {
                output_device: Some("PipeWire:alsa_output.usb".into()),
                echo_cancellation: false,
                noise_suppression: NoiseSuppression::Off,
                automatic_gain: false,
                applications: AudioApplications {
                    mode: AudioMode::Only,
                    names: vec!["firefox".into(), "game.exe".into()],
                    ..Default::default()
                },
                ..Default::default()
            },
            recent_rooms: vec![RecentRoom {
                ticket: "brpticket".into(),
                last_joined_unix: 1_788_000_000,
            }],
            check_updates: false,
            auto_join_links: true,
            auto_join_voice: true,
        }
    }

    #[test]
    fn defaults_and_every_field_round_trip_through_toml() {
        for settings in [Settings::default(), full()] {
            let text = toml::to_string_pretty(&settings).unwrap();
            let back: Settings = toml::from_str(&text).unwrap();
            assert_eq!(back, settings, "{text}");
        }
    }

    #[test]
    fn the_file_shape_matches_the_spec() {
        let text = toml::to_string_pretty(&full()).unwrap();
        assert!(text.contains("nickname = \"alice\""), "{text}");
        assert!(text.contains("[relay]"), "{text}");
        assert!(text.contains("mode = \"custom\""), "{text}");
        assert!(
            text.contains("url = \"https://relay.example.com/\""),
            "{text}"
        );
        assert!(text.contains("[audio]"), "{text}");
        assert!(text.contains("[audio.applications]"), "{text}");
        assert!(text.contains("mode = \"only\""), "{text}");
        assert!(text.contains("\"firefox\""), "{text}");
        assert!(text.contains("[[recent_rooms]]"), "{text}");
        assert!(text.contains("check_updates = false"), "{text}");
        let disabled = toml::to_string_pretty(&Settings {
            relay: RelayChoice::Disabled,
            ..Settings::default()
        })
        .unwrap();
        assert!(disabled.contains("mode = \"disabled\""), "{disabled}");
    }

    #[test]
    fn unknown_and_missing_keys_are_tolerated() {
        let settings: Settings =
            toml::from_str("fps = 24\nfuture_key = 1\n[relay]\nmode = \"disabled\"\n").unwrap();
        assert_eq!(settings.fps, 24);
        assert_eq!(settings.relay, RelayChoice::Disabled);
        assert_eq!(settings.nickname, None);
        assert!(settings.recent_rooms.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn saving_leaves_the_file_readable_only_by_its_owner_even_over_a_more_open_one() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_path("private");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "fps = 30\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        Settings::default().save(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "it holds tickets, and a ticket is enough to join"
        );
    }

    #[test]
    fn a_file_from_before_the_setting_still_asks_before_a_link_joins() {
        let settings: Settings = toml::from_str("fps = 24\n").unwrap();
        assert!(!settings.auto_join_links);
    }

    #[test]
    fn an_older_settings_file_reads_voice_as_off_with_default_devices() {
        let settings: Settings =
            toml::from_str("fps = 24\n[audio]\noutput_device = \"x\"\n").unwrap();
        assert!(!settings.auto_join_voice);
        assert_eq!(settings.audio.input_device, None);
        assert_eq!(settings.audio.voice_output_device, None);
        assert!(!settings.audio.separate_voice_output);
    }

    #[test]
    fn an_older_settings_file_turns_voice_processing_on() {
        let settings: Settings = toml::from_str("[audio]\noutput_device = \"x\"\n").unwrap();
        assert!(settings.audio.echo_cancellation);
        assert_eq!(settings.audio.noise_suppression, NoiseSuppression::Light);
        assert!(settings.audio.automatic_gain);
    }

    #[test]
    fn voice_processing_follows_the_three_settings() {
        let audio = AudioSettings {
            echo_cancellation: false,
            noise_suppression: NoiseSuppression::Off,
            automatic_gain: true,
            ..Default::default()
        };
        assert_eq!(
            audio.voice_processing(),
            ProcessingConfig {
                echo_cancellation: false,
                noise_suppression: Suppression::Off,
                automatic_gain: true,
            }
        );
        assert_eq!(
            AudioSettings::default()
                .voice_processing()
                .noise_suppression,
            Suppression::Light
        );
    }

    #[test]
    fn voice_plays_through_the_audio_output_unless_separated() {
        let mut audio = AudioSettings {
            output_device: Some("speakers".into()),
            voice_output_device: Some("headset".into()),
            ..Default::default()
        };
        assert_eq!(audio.voice_output().as_deref(), Some("speakers"));
        audio.separate_voice_output = true;
        assert_eq!(audio.voice_output().as_deref(), Some("headset"));
    }

    #[test]
    fn a_missing_file_loads_the_defaults() {
        let path = temp_path("missing");
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());
    }

    #[test]
    fn a_file_from_before_the_updater_checks_for_updates() {
        let settings: Settings = toml::from_str("fps = 24\n").unwrap();
        assert!(settings.check_updates);
        assert!(Settings::default().check_updates);
    }

    #[test]
    fn an_invalid_file_is_an_error_and_is_left_untouched() {
        let path = temp_path("invalid");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "this = = is not toml").unwrap();
        let error = Settings::load(&path).unwrap_err();
        assert!(matches!(error, AppError::Settings(_)), "{error}");
        assert!(
            error.to_string().contains(&path.display().to_string()),
            "{error}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "this = = is not toml");
        let store = SettingsStore::load_at(path);
        assert_eq!(store.settings, Settings::default());
        assert!(store.load_error.is_some());
    }

    #[test]
    fn a_file_with_an_invalid_relay_url_loads_as_an_error_and_is_left_untouched() {
        let path = temp_path("invalid_relay");
        let original = "[relay]\nmode = \"custom\"\nurl = \"nope\"\n";
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, original).unwrap();
        let error = Settings::load(&path).unwrap_err();
        assert!(matches!(error, AppError::Settings(_)), "{error}");
        assert!(
            error.to_string().contains(&path.display().to_string()),
            "{error}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        let store = SettingsStore::load_at(path);
        assert_eq!(store.settings, Settings::default());
        assert!(store.load_error.is_some());
    }

    #[test]
    fn save_creates_the_directory_replaces_the_file_and_leaves_no_temp_file() {
        let path = temp_path("save");
        Settings::default().save(&path).unwrap();
        full().save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), full());
        let siblings: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(siblings, [SETTINGS_FILE]);
        let mut store = SettingsStore::load_at(path.clone());
        store.load_error = Some("stale".into());
        store.save().unwrap();
        assert_eq!(store.load_error, None);
    }

    #[test]
    fn a_failed_load_blocks_implicit_saves_until_the_error_is_cleared() {
        let path = temp_path("save_unless_load_failed");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = "this = = is not toml";
        fs::write(&path, original).unwrap();
        let mut store = SettingsStore::load_at(path.clone());
        assert!(store.load_error.is_some());

        store.settings.remember_room("t", 1);
        assert!(!store.save_unless_load_failed().unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert!(store.load_error.is_some());

        store.load_error = None;
        assert!(store.save_unless_load_failed().unwrap());
        assert_eq!(Settings::load(&path).unwrap(), store.settings);
    }

    #[test]
    fn replacing_a_room_keeps_one_entry_for_it_and_leaves_the_others() {
        let mut settings = Settings::default();
        settings.remember_room("other", 1);
        settings.remember_room("old", 2);
        settings.remember_room("another", 3);
        settings.replace_room("old", "new", 4);
        let tickets: Vec<&str> = settings
            .recent_rooms
            .iter()
            .map(|room| room.ticket.as_str())
            .collect();
        assert_eq!(tickets, ["new", "another", "other"]);
        assert_eq!(settings.recent_rooms[0].last_joined_unix, 4);
    }

    #[test]
    fn remember_room_moves_a_repeat_to_the_front_and_caps_the_list() {
        let mut settings = Settings::default();
        for i in 0..RECENT_ROOMS_MAX as u64 + 2 {
            settings.remember_room(&format!("t{i}"), i);
        }
        assert_eq!(settings.recent_rooms.len(), RECENT_ROOMS_MAX);
        assert_eq!(settings.recent_rooms[0].ticket, "t9");
        assert!(
            settings
                .recent_rooms
                .iter()
                .all(|r| r.ticket != "t0" && r.ticket != "t1")
        );
        settings.remember_room("t5", 100);
        assert_eq!(settings.recent_rooms.len(), RECENT_ROOMS_MAX);
        assert_eq!(
            settings.recent_rooms[0],
            RecentRoom {
                ticket: "t5".into(),
                last_joined_unix: 100
            }
        );
        assert_eq!(
            settings
                .recent_rooms
                .iter()
                .filter(|r| r.ticket == "t5")
                .count(),
            1
        );
    }

    #[test]
    fn validate_rejects_a_zero_frame_rate_and_a_bad_relay_url() {
        let mut settings = Settings::default();
        assert_eq!(settings.validate(), Ok(()));
        settings.fps = 0;
        assert!(settings.validate().unwrap_err().contains("frame rate"));
        settings.fps = 60;
        settings.relay = RelayChoice::Custom("not a url".into());
        assert!(settings.validate().unwrap_err().contains("relay URL"));
        settings.relay = RelayChoice::Custom("https://relay.example.com/".into());
        assert_eq!(settings.validate(), Ok(()));
    }

    #[test]
    fn normalised_nickname_drops_whitespace_and_treats_blank_as_none() {
        assert_eq!(normalised_nickname(""), None);
        assert_eq!(normalised_nickname("   "), None);
        assert_eq!(
            normalised_nickname("  John Smith "),
            Some("John Smith".to_string())
        );
    }

    #[test]
    fn relay_choice_maps_onto_the_transport_setting() {
        assert_eq!(
            RelayChoice::Default.to_relay_setting().unwrap(),
            RelaySetting::Default
        );
        assert_eq!(
            RelayChoice::Disabled.to_relay_setting().unwrap(),
            RelaySetting::Disabled
        );
        let url: RelayUrl = "https://relay.example.com/".parse().unwrap();
        assert_eq!(
            RelayChoice::Custom("https://relay.example.com/".into())
                .to_relay_setting()
                .unwrap(),
            RelaySetting::Custom(url)
        );
        assert!(
            RelayChoice::Custom("nope".into())
                .to_relay_setting()
                .is_err()
        );
    }

    #[test]
    fn the_mode_selects_all_or_only_the_named_applications() {
        assert_eq!(
            AudioApplications::default().to_selection(),
            AudioSelection::All
        );
        let only = AudioApplications {
            mode: AudioMode::Only,
            names: vec!["firefox".into(), "game.exe".into()],
            ..Default::default()
        };
        assert_eq!(
            only.to_selection(),
            AudioSelection::Only(BTreeSet::from([
                AppKey::new("firefox"),
                AppKey::new("game.exe"),
            ]))
        );
        let kept = AudioApplications {
            mode: AudioMode::All,
            names: vec!["firefox".into()],
            ..Default::default()
        };
        assert_eq!(
            kept.to_selection(),
            AudioSelection::All,
            "the names are kept in both modes but only in force under Only"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_hand_edited_name_is_normalised_by_the_conversion() {
        let odd = AudioApplications {
            mode: AudioMode::Only,
            names: vec![r"C:\Games\GAME.EXE".into()],
            ..Default::default()
        };
        assert_eq!(
            odd.to_selection(),
            AudioSelection::Only(BTreeSet::from([AppKey::new("game.exe")])),
            "the conversion goes through AppKey, so odd casing and a full path still match"
        );
    }

    #[test]
    fn an_unknown_application_mode_fails_the_parse() {
        let settings: Result<Settings, _> =
            toml::from_str("[audio.applications]\nmode = \"sometimes\"\n");
        assert!(settings.is_err());
    }

    #[test]
    fn a_file_without_an_applications_table_shares_every_application() {
        let settings: Settings = toml::from_str("[audio]\noutput_device = \"x\"\n").unwrap();
        assert_eq!(settings.audio.applications, AudioApplications::default());
        assert_eq!(
            settings.audio.applications.to_selection(),
            AudioSelection::All
        );
    }
}
