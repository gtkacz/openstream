//! Opening a room from the window: the settings the command line provides, and the async open
//! that the start screen and the `create`/`join` commands share.

use std::sync::Arc;

use brp_audio::{AudioSelection, CpalOutput, PlatformAudioCapture};
use brp_capture::PlatformCapture;
use brp_net::RelaySetting;
use brp_proto::RoomTicket;
use brp_proto::constants::RELAY_ONLINE_TIMEOUT;
use brp_room::codecs::FfmpegCodecs;
use brp_room::{Room, RoomConfig, RoomTimings};
use iroh::SecretKey;
use winit::event_loop::EventLoopProxy;

use crate::cli::WindowArgs;
use crate::error::AppError;
use crate::settings::Settings;
use crate::window::AppEvent;

/// What the user asked for: a fresh room, or a seat in an existing one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Create,
    Join(RoomTicket),
}

/// Settings that apply to any room this window opens: the saved settings, with each command line
/// flag that is present replacing its field for this launch only.
#[derive(Debug, Clone)]
pub struct Launch {
    pub nickname: Option<String>,
    /// True when `--nickname` was given: the start screen's nickname is then not saved.
    pub nickname_from_flag: bool,
    pub fps: u32,
    pub relay: RelaySetting,
    /// cpal device id to play through; `None` is the system default.
    pub audio_output: Option<String>,
    /// Which applications the room's audio carries.
    pub audio_applications: AudioSelection,
}

impl Launch {
    pub fn from_settings(settings: &Settings, args: &WindowArgs) -> Result<Self, AppError> {
        let relay = if args.no_relay {
            RelaySetting::Disabled
        } else {
            settings.relay.to_relay_setting()?
        };
        Ok(Self {
            nickname: args.nickname.clone().or_else(|| settings.nickname.clone()),
            nickname_from_flag: args.nickname.is_some(),
            fps: args.fps.unwrap_or(settings.fps),
            relay,
            audio_output: settings.audio.output_device.clone(),
            audio_applications: settings.audio.applications.to_selection(),
        })
    }
}

/// The nickname the start screen offers: the `--nickname` flag, else the saved nickname, else
/// the short peer id.
pub fn default_nickname(launch: &Launch, secret: &SecretKey) -> String {
    match &launch.nickname {
        Some(nickname) => nickname.clone(),
        None => secret.public().fmt_short().to_string(),
    }
}

/// Creates or joins the room and waits briefly for the relay so the ticket works off the LAN.
/// A blank `nickname` falls back to the short peer id. The ticket is not printed or logged.
pub async fn open_room(
    launch: &Launch,
    secret: SecretKey,
    intent: Intent,
    nickname: &str,
    proxy: EventLoopProxy<AppEvent>,
) -> Result<Arc<Room>, AppError> {
    let nickname = match nickname.trim() {
        "" => secret.public().fmt_short().to_string(),
        name => name.to_string(),
    };
    let change_proxy = proxy.clone();
    let config = RoomConfig {
        secret,
        relay: launch.relay.clone(),
        nickname,
        target_fps: launch.fps,
        capture: Arc::new(PlatformCapture),
        audio_capture: Arc::new(PlatformAudioCapture::new(std::process::id())),
        audio_output: Arc::new(CpalOutput::new(launch.audio_output.clone())),
        audio_applications: launch.audio_applications.clone(),
        encoders: Arc::new(FfmpegCodecs::default()),
        decoders: Arc::new(FfmpegCodecs::default()),
        on_change: Arc::new(move || {
            let _ = change_proxy.send_event(AppEvent::RoomChanged);
        }),
        on_frame: Arc::new(move || {
            let _ = proxy.send_event(AppEvent::NewFrame);
        }),
        timings: RoomTimings::default(),
    };
    let room = match intent {
        Intent::Join(ticket) => Room::join(config, ticket).await?,
        Intent::Create => Room::create(config).await?,
    };
    if launch.relay != RelaySetting::Disabled && !room.online(RELAY_ONLINE_TIMEOUT).await {
        tracing::warn!(
            "relay registration timed out; the ticket may only work on the local network"
        );
    }
    Ok(Arc::new(room))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use crate::settings::{AudioApplications, AudioMode, AudioSettings, RelayChoice};
    use brp_audio::AppKey;

    fn saved() -> Settings {
        Settings {
            nickname: Some("saved".into()),
            fps: 30,
            relay: RelayChoice::Custom("https://relay.example.com/".into()),
            audio: AudioSettings {
                output_device: Some("dev".into()),
                applications: AudioApplications {
                    mode: AudioMode::Only,
                    names: vec!["firefox".into(), "game.exe".into()],
                },
            },
            recent_rooms: Vec::new(),
        }
    }

    #[test]
    fn without_flags_the_saved_settings_apply() {
        let launch = Launch::from_settings(&saved(), &WindowArgs::default()).unwrap();
        assert_eq!(launch.nickname.as_deref(), Some("saved"));
        assert!(!launch.nickname_from_flag);
        assert_eq!(launch.fps, 30);
        assert!(matches!(launch.relay, RelaySetting::Custom(_)));
        assert_eq!(launch.audio_output.as_deref(), Some("dev"));
        assert_eq!(
            launch.audio_applications,
            AudioSelection::Only(BTreeSet::from([
                AppKey::new("firefox"),
                AppKey::new("game.exe"),
            ]))
        );
    }

    #[test]
    fn each_flag_overrides_only_its_field() {
        let args = WindowArgs {
            nickname: Some("flag".into()),
            fps: Some(120),
            no_relay: true,
        };
        let launch = Launch::from_settings(&saved(), &args).unwrap();
        assert_eq!(launch.nickname.as_deref(), Some("flag"));
        assert!(launch.nickname_from_flag);
        assert_eq!(launch.fps, 120);
        assert_eq!(launch.relay, RelaySetting::Disabled);
        assert_eq!(launch.audio_output.as_deref(), Some("dev"));
        assert_eq!(
            launch.audio_applications,
            AudioSelection::Only(BTreeSet::from([
                AppKey::new("firefox"),
                AppKey::new("game.exe"),
            ]))
        );
    }

    #[test]
    fn a_bad_saved_relay_url_is_an_error_unless_relays_are_off() {
        let mut settings = saved();
        settings.relay = RelayChoice::Custom("nope".into());
        assert!(Launch::from_settings(&settings, &WindowArgs::default()).is_err());
        let args = WindowArgs {
            no_relay: true,
            ..WindowArgs::default()
        };
        assert_eq!(
            Launch::from_settings(&settings, &args).unwrap().relay,
            RelaySetting::Disabled
        );
    }
}
