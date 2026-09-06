//! `brp create` and `brp join`: a room participant with the window. Owns the room's lifetime
//! around the winit loop and tears it down when the window closes.

use std::str::FromStr;
use std::sync::Arc;

use brp_proto::RoomTicket;
use brp_proto::constants::STATS_LOG_INTERVAL;
use tokio::runtime::Runtime;
use winit::event_loop::EventLoop;

use crate::cli::WindowArgs;
use crate::error::AppError;
use crate::launch::{self, Intent, Launch};
use crate::window::{App, AppEvent};

/// Runs `brp create` or `brp join` to completion: creates or joins the room, opens the
/// participant window, and blocks until the window closes, at which point the room is left in an
/// orderly fashion. `ticket` selects join over create.
pub fn run(runtime: &Runtime, ticket: Option<String>, args: WindowArgs) -> Result<(), AppError> {
    let intent = match ticket.as_deref().map(RoomTicket::from_str).transpose()? {
        Some(ticket) => Intent::Join(ticket),
        None => Intent::Create,
    };
    let launch = Launch::from(args);
    let nickname = launch::default_nickname(&launch)?;

    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|e| AppError::Window(e.to_string()))?;
    let proxy = event_loop.create_proxy();

    let room = runtime.block_on(launch::open_room(&launch, intent, &nickname, proxy.clone()))?;

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
