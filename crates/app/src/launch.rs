//! Opening a room from the window: the settings the command line provides, and the async open
//! that the start screen and the `create`/`join` commands share.

use std::sync::Arc;

use brp_capture::PlatformCapture;
use brp_net::RelaySetting;
use brp_proto::RoomTicket;
use brp_proto::constants::RELAY_ONLINE_TIMEOUT;
use brp_room::codecs::FfmpegCodecs;
use brp_room::{Room, RoomConfig, RoomTimings};
use winit::event_loop::EventLoopProxy;

use crate::cli::WindowArgs;
use crate::error::AppError;
use crate::identity;
use crate::window::AppEvent;

/// What the user asked for: a fresh room, or a seat in an existing one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Create,
    Join(RoomTicket),
}

/// Settings that apply to any room this window opens.
#[derive(Debug, Clone)]
pub struct Launch {
    pub nickname: Option<String>,
    pub fps: u32,
    pub relay: RelaySetting,
}

impl From<WindowArgs> for Launch {
    fn from(args: WindowArgs) -> Self {
        Self {
            nickname: args.nickname,
            fps: args.fps,
            relay: if args.no_relay {
                RelaySetting::Disabled
            } else {
                RelaySetting::Default
            },
        }
    }
}

/// The nickname the start screen offers: the `--nickname` flag, else the short peer id.
pub fn default_nickname(launch: &Launch) -> Result<String, AppError> {
    match &launch.nickname {
        Some(nickname) => Ok(nickname.clone()),
        None => Ok(identity::load_or_create()?.public().fmt_short().to_string()),
    }
}

/// Creates or joins the room and waits briefly for the relay so the ticket works off the LAN.
/// A blank `nickname` falls back to the short peer id. The ticket is not printed or logged.
pub async fn open_room(
    launch: &Launch,
    intent: Intent,
    nickname: &str,
    proxy: EventLoopProxy<AppEvent>,
) -> Result<Arc<Room>, AppError> {
    let secret = identity::load_or_create()?;
    let nickname = match nickname.trim() {
        "" => secret.public().fmt_short().to_string(),
        name => name.to_string(),
    };
    let change_proxy = proxy.clone();
    let config = RoomConfig {
        secret,
        relay: launch.relay,
        nickname,
        target_fps: launch.fps,
        capture: Arc::new(PlatformCapture),
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
    if launch.relay == RelaySetting::Default && !room.online(RELAY_ONLINE_TIMEOUT).await {
        tracing::warn!(
            "relay registration timed out; the ticket may only work on the local network"
        );
    }
    Ok(Arc::new(room))
}
