//! The participant window: `brp` with no arguments shows the start screen, `brp create` and
//! `brp join` open a room at once. Owns the room's lifetime around the winit loop and leaves it
//! in an orderly fashion when the window closes.

use std::sync::Arc;

use brp_proto::constants::STATS_LOG_INTERVAL;
use tokio::runtime::Runtime;
use winit::event_loop::EventLoop;

use crate::cli::WindowArgs;
use crate::error::AppError;
use crate::identity;
use crate::launch::{self, Intent, Launch};
use crate::window::{App, AppEvent};

/// Runs the window to completion. `intent` from the command line opens the room immediately;
/// `None` shows the start screen.
pub fn run(runtime: &Runtime, intent: Option<Intent>, args: WindowArgs) -> Result<(), AppError> {
    let launch = Launch::from(args);
    let secret = identity::load_or_create()?;
    let nickname = launch::default_nickname(&launch, &secret);

    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|e| AppError::Window(e.to_string()))?;
    let proxy = event_loop.create_proxy();

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

    let mut app = App::new(
        runtime.handle().clone(),
        proxy,
        launch,
        secret,
        nickname,
        intent,
    );
    let outcome = event_loop
        .run_app(&mut app)
        .map_err(|e| AppError::Window(e.to_string()));

    ticker.abort();
    let shutdown = app.finish();
    // Abort only requests cancellation; wait for every share task so its Arc<Room> clone is gone
    // before the room is unwrapped (a cancelled JoinError is expected).
    let _ = runtime.block_on(ticker);
    for task in shutdown.tasks {
        task.abort();
        let _ = runtime.block_on(task);
    }
    let mut rooms = shutdown.room.into_iter().collect::<Vec<_>>();
    // An open still in flight is awaited, not aborted: aborting after the room exists would drop
    // it without a leave. Closing the window during a doomed join therefore waits out the join
    // and relay timeouts before the process exits.
    if let Some(open) = shutdown.pending_open
        && let Ok(Ok(room)) = runtime.block_on(open)
    {
        rooms.push(room);
    }
    for room in rooms {
        match Arc::try_unwrap(room) {
            Ok(room) => runtime.block_on(room.leave()),
            Err(_) => tracing::warn!("room still referenced at exit; skipping the orderly leave"),
        }
    }
    outcome
}
