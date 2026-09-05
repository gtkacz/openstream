use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use brp_capture::PortalCapture;
use brp_net::RelaySetting;
use brp_proto::RoomTicket;
use brp_proto::constants::{JOIN_TIMEOUT, SOURCE_PRESET_ID};
use brp_room::codecs::FfmpegCodecs;
use brp_room::{Room, RoomConfig, RoomError, RoomTimings, WatchState};
use tokio::runtime::Runtime;
use winit::event_loop::EventLoop;

use crate::cli::WatchArgs;
use crate::error::AppError;
use crate::identity;
use crate::window::{App, AppEvent};

const LIVE_ID: u32 = 1;
/// How often the status line follows the watch state until plan 2b renders snapshots directly.
const STATUS_POLL: Duration = Duration::from_millis(500);

pub fn run(runtime: &Runtime, args: WatchArgs) -> Result<(), AppError> {
    let ticket = RoomTicket::from_str(&args.ticket)?;
    let publisher = ticket.bootstrap.first().ok_or(AppError::EmptyTicket)?.id;
    let relay = if args.no_relay {
        RelaySetting::Disabled
    } else {
        RelaySetting::Default
    };

    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|e| AppError::Window(e.to_string()))?;
    let proxy = event_loop.create_proxy();
    let frame_proxy = proxy.clone();

    let (room, handle, description) = runtime.block_on(async {
        let secret = identity::load_or_create()?;
        let nickname = args
            .nickname
            .clone()
            .unwrap_or_else(|| secret.public().fmt_short().to_string());
        let config = RoomConfig {
            secret,
            relay,
            nickname,
            target_fps: 60,
            capture: Arc::new(PortalCapture),
            encoders: Arc::new(FfmpegCodecs::default()),
            decoders: Arc::new(FfmpegCodecs::default()),
            on_change: Arc::new(|| {}),
            on_frame: Arc::new(move || {
                let _ = frame_proxy.send_event(AppEvent::NewFrame);
            }),
            timings: RoomTimings::default(),
        };
        let room = Room::join(config, ticket).await?;
        let deadline = Instant::now() + JOIN_TIMEOUT;
        let live = loop {
            let found = room
                .snapshot()
                .members
                .into_iter()
                .find(|m| m.id == publisher)
                .and_then(|m| m.lives.into_iter().find(|l| l.id == LIVE_ID));
            if let Some(live) = found {
                break live;
            }
            if Instant::now() > deadline {
                return Err(AppError::Room(RoomError::UnknownLive(LIVE_ID)));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let handle = room.watch(publisher, LIVE_ID, SOURCE_PRESET_ID)?;
        let description = format!(
            "{} {}x{} @ {} fps from {}",
            live.title,
            live.source_width,
            live.source_height,
            live.source_fps,
            publisher.fmt_short()
        );
        Ok::<_, AppError>((Arc::new(room), handle, description))
    })?;
    println!("Watching: {description}");

    let poller = runtime.spawn({
        let room = room.clone();
        let mut last = String::new();
        async move {
            let mut tick = tokio::time::interval(STATUS_POLL);
            loop {
                tick.tick().await;
                let snapshot = room.snapshot();
                let status = match snapshot.watches.first() {
                    Some(w) if w.state == WatchState::Live => format!(
                        "live, {:?} path",
                        snapshot
                            .members
                            .iter()
                            .find(|m| m.id == w.publisher)
                            .map(|m| m.path)
                    ),
                    Some(w) => format!("{:?}", w.state),
                    None => "publisher left the room".to_string(),
                };
                if status != last && proxy.send_event(AppEvent::Status(status.clone())).is_ok() {
                    last = status;
                }
            }
        }
    });

    let mut app = App::new(
        format!("brp: {}", publisher.fmt_short()),
        description,
        handle.slot.clone(),
        handle.stats.clone(),
    );
    let outcome = event_loop
        .run_app(&mut app)
        .map_err(|e| AppError::Window(e.to_string()));

    poller.abort();
    // try_unwrap below needs the poller's Arc<Room> clone gone; abort only requests
    // cancellation, so wait for the task to actually finish (a cancelled JoinError is expected).
    let _ = runtime.block_on(poller);
    drop(handle);
    if let Ok(room) = Arc::try_unwrap(room) {
        runtime.block_on(room.leave());
    }
    outcome
}
