//! The participant window: `brp` with no arguments shows the start screen, `brp create` and
//! `brp join` open a room at once. Owns the room's lifetime around the winit loop and leaves it
//! in an orderly fashion when the window closes.

use std::str::FromStr;
use std::sync::Arc;

use brp_proto::constants::STATS_LOG_INTERVAL;
use brp_update::{Install, Version};
use tokio::runtime::Runtime;
use winit::event_loop::{EventLoop, EventLoopProxy};

use crate::cli::WindowArgs;
use crate::error::AppError;
use crate::handler;
use crate::identity;
use crate::launch::{self, Intent, Launch};
use crate::settings::SettingsStore;
use crate::window::{App, AppEvent};

/// Runs the window to completion. `intent` from the command line opens the room immediately;
/// `None` shows the start screen, with `start_error` on it when the command line had a ticket
/// that did not parse.
pub fn run(
    runtime: &Runtime,
    intent: Option<Intent>,
    start_error: Option<String>,
    args: WindowArgs,
) -> Result<(), AppError> {
    handler::register_scheme_handler();
    let store = SettingsStore::load()?;
    let launch = Launch::from_settings(&store.settings, &args)?;
    let secret = identity::load_or_create()?;
    let nickname = launch::default_nickname(&launch, &secret);
    let install = match Install::current() {
        Ok(install) => {
            // Only an update can have left those files, and an update only ever runs in a release
            // layout; beside a dev build or a hand-copied binary they belong to someone else.
            if install.is_release_layout() {
                brp_update::cleanup_stale(&install);
            }
            Some(install)
        }
        Err(error) => {
            tracing::warn!(%error, "install path unknown; updates can be noticed but not applied");
            None
        }
    };

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

    if store.settings.check_updates {
        spawn_update_check(runtime, proxy.clone());
    }

    let mut app = App::new(
        runtime.handle().clone(),
        proxy,
        args,
        secret,
        nickname,
        intent,
        start_error,
        store,
        install,
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
    if let Some(relaunch) = shutdown.relaunch {
        relaunch.spawn();
    }
    outcome
}

/// Asks GitHub once whether a newer release exists. Offline is normal, so a failure is a log
/// line and nothing in the window.
fn spawn_update_check(runtime: &Runtime, proxy: EventLoopProxy<AppEvent>) {
    let current = match Version::from_str(env!("CARGO_PKG_VERSION")) {
        Ok(current) => current,
        Err(error) => {
            tracing::warn!(%error, "not checking for updates: this build's version is not a release version");
            return;
        }
    };
    runtime.spawn(async move {
        match brp_update::check(current).await {
            Ok(Some(release)) => {
                let _ = proxy.send_event(AppEvent::UpdateAvailable(release));
            }
            Ok(None) => tracing::debug!("this is the latest release"),
            Err(error) => tracing::warn!(%error, "update check failed"),
        }
    });
}
