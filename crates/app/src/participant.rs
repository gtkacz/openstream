//! `brp create` and `brp join`: a room participant with the window. Owns the room's lifetime
//! around the winit loop and tears it down when the window closes.

use std::str::FromStr;
use std::sync::Arc;

use brp_capture::PortalCapture;
use brp_net::RelaySetting;
use brp_proto::RoomTicket;
use brp_proto::constants::{RELAY_ONLINE_TIMEOUT, STATS_LOG_INTERVAL};
use brp_room::codecs::FfmpegCodecs;
use brp_room::{Room, RoomConfig, RoomTimings};
use tokio::runtime::Runtime;
use winit::event_loop::EventLoop;

use crate::cli::WindowArgs;
use crate::error::AppError;
use crate::identity;
use crate::window::{App, AppEvent};

/// Runs `brp create` or `brp join` to completion: creates or joins the room, opens the
/// participant window, and blocks until the window closes, at which point the room is left in an
/// orderly fashion. `ticket` selects join over create.
pub fn run(runtime: &Runtime, ticket: Option<String>, args: WindowArgs) -> Result<(), AppError> {
    let ticket = ticket.as_deref().map(RoomTicket::from_str).transpose()?;
    let relay = if args.no_relay {
        RelaySetting::Disabled
    } else {
        RelaySetting::Default
    };

    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|e| AppError::Window(e.to_string()))?;
    let proxy = event_loop.create_proxy();
    let change_proxy = proxy.clone();
    let frame_proxy = proxy.clone();

    let room = runtime.block_on(async {
        let secret = identity::load_or_create()?;
        let nickname = args
            .nickname
            .clone()
            .unwrap_or_else(|| secret.public().fmt_short().to_string());
        let config = RoomConfig {
            secret,
            relay,
            nickname,
            target_fps: args.fps,
            capture: Arc::new(PortalCapture),
            encoders: Arc::new(FfmpegCodecs::default()),
            decoders: Arc::new(FfmpegCodecs::default()),
            on_change: Arc::new(move || {
                let _ = change_proxy.send_event(AppEvent::RoomChanged);
            }),
            on_frame: Arc::new(move || {
                let _ = frame_proxy.send_event(AppEvent::NewFrame);
            }),
            timings: RoomTimings::default(),
        };
        let room = match ticket {
            Some(ticket) => Room::join(config, ticket).await?,
            None => Room::create(config).await?,
        };
        if relay == RelaySetting::Default && !room.online(RELAY_ONLINE_TIMEOUT).await {
            tracing::warn!(
                "relay registration timed out; the ticket may only work on the local network"
            );
        }
        Ok::<_, AppError>(Arc::new(room))
    })?;
    println!("Ticket:\n{}\n", room.ticket());

    // Encoder byte counters and last-seen ages move without any frame arriving, so a slow tick
    // keeps the status bar honest when nothing is watched.
    let ticker = runtime.spawn({
        let proxy = proxy.clone();
        async move {
            let mut tick = tokio::time::interval(STATS_LOG_INTERVAL);
            loop {
                tick.tick().await;
                if proxy.send_event(AppEvent::Tick).is_err() {
                    break;
                }
            }
        }
    });

    let mut app = App::new(runtime.handle().clone(), room.clone(), proxy);
    let outcome = event_loop
        .run_app(&mut app)
        .map_err(|e| AppError::Window(e.to_string()));

    ticker.abort();
    let pending_share = app.take_pending_share();
    drop(app);
    // Abort only requests cancellation; wait for both tasks so their Arc<Room> clones are gone
    // before the room is unwrapped (a cancelled JoinError is expected).
    let _ = runtime.block_on(ticker);
    if let Some(task) = pending_share {
        task.abort();
        let _ = runtime.block_on(task);
    }
    match Arc::try_unwrap(room) {
        Ok(room) => runtime.block_on(room.leave()),
        Err(_) => tracing::warn!("room still referenced at exit; skipping the orderly leave"),
    }
    outcome
}
