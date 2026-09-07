//! The participant windows: a winit loop that shows the start screen until a room is open, then
//! draws the tile grid under the egui panels in the main window and one live per pop-out window.
//! Panel commands go to the room view; window commands are applied here.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use brp_room::Room;
use iroh::SecretKey;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy},
    window::{Fullscreen, Window, WindowId},
};

use crate::cli::WindowArgs;
use crate::commands::WindowCommand;
use crate::launch::{self, Intent, Launch};
use crate::popouts::PopOuts;
use crate::render::GpuContext;
use crate::render::grid::{self, PixelRect};
use crate::render::surface::WindowSurface;
use crate::render::tiles::{TileKey, TileRenderer};
use crate::render::ui::UiFrame;
use crate::room_view::RoomView;
use crate::settings::{SettingsStore, normalised_nickname, now_unix};
use crate::ui::settings::{self as settings_ui, SettingsDialog};
use crate::ui::start::{self, StartAction, StartState};
use crate::ui::state::{UiState, live_title};
use crate::ui::{self, UiOutput, popout};

/// Initial inner size of the main window and of every pop-out, in physical pixels.
pub const DEFAULT_WINDOW_SIZE: PhysicalSize<u32> = PhysicalSize::new(1280, 720);

/// Wakes the winit event loop for a reason that does not arrive as a `WindowEvent`. Sent through
/// the `EventLoopProxy` from other threads (the room's background tasks, the open and share tasks).
pub enum AppEvent {
    /// The room's version counter moved; re-snapshot on the next redraw.
    RoomChanged,
    /// A watched live decoded a frame.
    NewFrame,
    /// Periodic wake so counters refresh while nothing is watched.
    Tick,
    /// The share task finished: the live started, or the error to show.
    ShareFinished(Result<(), String>),
    /// The open task finished: the room to show, or the error for the start screen.
    RoomOpened(Result<Arc<Room>, String>),
}

/// What the window shows: the start screen, or a room.
enum Phase {
    Start,
    // Boxed: `RoomView` is much larger than `Start`, and clippy's large_enum_variant lint
    // treats the size gap as a wasted-space signal for every `Phase` on the stack.
    Room(Box<RoomView>),
}

/// A live shown in a window of its own. `fullscreen` is the state we asked winit for; the label
/// follows it even if the compositor refused, so the user can ask again.
struct PopOutWindow {
    surface: WindowSurface,
    fullscreen: bool,
}

/// What must be torn down after the loop ends: share tasks still holding room handles, an open
/// that may still be producing a room, and the room itself.
pub struct Shutdown {
    pub room: Option<Arc<Room>>,
    pub tasks: Vec<JoinHandle<()>>,
    /// Awaited, never aborted: a room it produces after the window closed must still be left.
    pub pending_open: Option<JoinHandle<Result<Arc<Room>, String>>>,
}

/// The winit `ApplicationHandler` for the participant windows: owns the phase, the UI state
/// shared by every window, the shared GPU state, the main window, and the pop-outs.
pub struct App {
    runtime: Handle,
    proxy: EventLoopProxy<AppEvent>,
    /// The flags of this launch, which override the file each time a room opens.
    args: WindowArgs,
    /// Loaded once at startup; every room this window opens signs as this identity.
    secret: SecretKey,
    start: StartState,
    phase: Phase,
    state: UiState,
    pending_open: Option<JoinHandle<Result<Arc<Room>, String>>>,
    store: SettingsStore,
    settings_dialog: SettingsDialog,
    /// The intent an open in flight was started with; a join remembers the ticket that got us in,
    /// a create remembers the room's own ticket.
    pending_intent: Option<Intent>,
    /// The earliest instant any window's egui asked for its next frame; `about_to_wait` sleeps
    /// until then instead of forever.
    next_repaint: Option<Instant>,
    gpu: Option<GpuContext>,
    main: Option<WindowSurface>,
    /// Shared by every window: a frame is uploaded once whichever window shows it.
    tiles: Option<TileRenderer>,
    popouts: PopOuts<WindowId>,
    popout_windows: HashMap<WindowId, PopOutWindow>,
}

impl App {
    /// An `intent` from the command line opens the room at once behind the connecting start
    /// screen; `None` waits for the user.
    pub fn new(
        runtime: Handle,
        proxy: EventLoopProxy<AppEvent>,
        args: WindowArgs,
        secret: SecretKey,
        nickname: String,
        intent: Option<Intent>,
        store: SettingsStore,
    ) -> Self {
        let mut app = Self {
            runtime,
            proxy,
            args,
            secret,
            start: StartState::new(nickname),
            phase: Phase::Start,
            state: UiState::new(),
            pending_open: None,
            next_repaint: None,
            gpu: None,
            main: None,
            tiles: None,
            popouts: PopOuts::new(),
            popout_windows: HashMap::new(),
            store,
            settings_dialog: SettingsDialog::default(),
            pending_intent: None,
        };
        if let Some(message) = &app.store.load_error {
            app.start.error = format!("settings not loaded, defaults in use: {message}");
        }
        if let Some(intent) = intent {
            app.start.connecting = true;
            app.open(intent);
        }
        app
    }

    /// Consumes the app once the loop has ended and hands back what still holds a room, in the
    /// order the caller must tear it down: share tasks, the pending open, then the room itself.
    /// Pop-out windows drop with the app; nothing in them outlives the loop.
    pub fn finish(self) -> Shutdown {
        let (room, tasks) = match self.phase {
            Phase::Room(view) => (Some(view.room), view.pending_share.into_iter().collect()),
            Phase::Start => (None, Vec::new()),
        };
        Shutdown {
            room,
            tasks,
            pending_open: self.pending_open,
        }
    }

    /// Builds the launch from the current settings and the retained command line flags, so a
    /// dialog Save applies to the next room without a restart, then opens it in the background.
    fn open(&mut self, intent: Intent) {
        self.pending_intent = Some(intent.clone());
        let launch = match Launch::from_settings(&self.store.settings, &self.args) {
            Ok(launch) => launch,
            Err(error) => {
                self.start.failed(error.to_string());
                self.pending_intent = None;
                return;
            }
        };
        let secret = self.secret.clone();
        let nickname = self.start.nickname.clone();
        let room_events = self.proxy.clone();
        let done = self.proxy.clone();
        self.pending_open = Some(self.runtime.spawn(async move {
            let outcome = launch::open_room(&launch, secret, intent, &nickname, room_events)
                .await
                .map_err(|error| error.to_string());
            // The window learns of the outcome through the event; the task output is for the
            // shutdown path, which must leave a room that opened after the window closed.
            let _ = done.send_event(AppEvent::RoomOpened(outcome.clone()));
            outcome
        }));
    }

    /// Persists what a successful open teaches us: the ticket to list under recent rooms, and the
    /// nickname typed on the start screen unless a flag chose it. Failures are shown, not fatal.
    fn remember_open(&mut self, room: &Room) {
        let ticket = match self.pending_intent.take() {
            Some(Intent::Join(ticket)) => ticket.to_string(),
            Some(Intent::Create) | None => room.ticket().to_string(),
        };
        self.store.settings.remember_room(&ticket, now_unix());
        if self.args.nickname.is_none()
            && let Some(nickname) = normalised_nickname(&self.start.nickname)
        {
            self.store.settings.nickname = Some(nickname);
        }
        if let Err(error) = self.store.save_unless_load_failed() {
            self.state.status = format!("settings not saved: {error}");
        }
    }

    fn request_redraw_all(&self) {
        if let Some(main) = &self.main {
            main.window.request_redraw();
        }
        for popout in self.popout_windows.values() {
            popout.surface.window.request_redraw();
        }
    }

    fn is_main(&self, id: WindowId) -> bool {
        self.main.as_ref().is_some_and(|m| m.window.id() == id)
    }

    /// Keeps the earliest requested repaint across windows.
    fn note_repaint(&mut self, delay: Duration) {
        let deadline = repaint_deadline(Instant::now(), delay);
        self.next_repaint = match (self.next_repaint, deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }

    /// Re-snapshots the room and closes pop-outs whose watch has ended. Runs at the start of every
    /// redraw, whichever window asked, so all windows draw from the same snapshot.
    fn refresh_room(&mut self) {
        let Phase::Room(view) = &mut self.phase else {
            return;
        };
        view.refresh(&mut self.state, self.tiles.as_mut());
        let watched = view.watched_keys();
        let closed = self.popouts.retain_watched(&watched);
        if closed.is_empty() {
            return;
        }
        for id in closed {
            self.popout_windows.remove(&id);
        }
        if let Some(main) = &self.main {
            main.window.request_redraw();
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        self.refresh_room();
        if self.is_main(id) {
            self.redraw_main(event_loop);
        } else if self.popouts.key_of(id).is_some() {
            self.redraw_popout(event_loop, id);
        }
    }

    fn redraw_main(&mut self, event_loop: &ActiveEventLoop) {
        let (Some(gpu), Some(main), Some(tiles)) =
            (self.gpu.as_ref(), self.main.as_mut(), self.tiles.as_mut())
        else {
            return;
        };
        if let Phase::Room(view) = &self.phase {
            view.upload_frames(gpu, tiles);
        }
        let popped = self.popouts.popped();
        let size = main.size();
        let mut output = UiOutput::default();
        let mut start_action = None;
        let room_open = matches!(self.phase, Phase::Room(_));
        let now = now_unix();
        let mut saved = None;
        let mut ui_frame = main.ui.run(&main.window, [size.0, size.1], |root| {
            match &self.phase {
                Phase::Start => {
                    start_action = start::draw(
                        root,
                        &mut self.start,
                        &self.store.settings.recent_rooms,
                        now,
                    );
                }
                Phase::Room(view) => {
                    output = ui::draw(root, &view.snapshot, &view.ticket, &mut self.state, &popped);
                }
            }
            // egui may run this closure twice in a pass; a `None` from the second run must not
            // discard a `Some` the first run already produced.
            if let Some(settings) =
                settings_ui::draw(root.ctx(), &mut self.settings_dialog, room_open)
            {
                saved = Some(settings);
            }
        });
        let placements = pixel_placements(&output, ui_frame.screen.pixels_per_point, size);
        let presented = present(gpu, tiles, main, &mut ui_frame, &placements);
        let repaint_delay = ui_frame.repaint_delay;
        if repaint_delay.is_zero() {
            if presented {
                main.window.request_redraw();
            }
        } else {
            self.note_repaint(repaint_delay);
        }

        let had_commands = !output.commands.is_empty();
        let had_window_commands = !output.window_commands.is_empty();
        if let Some(action) = start_action
            && let Some(intent) = self.start.submit(action)
        {
            self.open(intent);
        }
        if let Phase::Room(view) = &mut self.phase
            && had_commands
        {
            view.apply(output.commands, &self.runtime, &self.proxy, &mut self.state);
        }
        if had_window_commands {
            self.apply_window_commands(event_loop, output.window_commands);
        }
        if (start_action.is_some() || had_commands || had_window_commands)
            && let Some(main) = &self.main
        {
            main.window.request_redraw();
        }

        if output.open_settings || start_action == Some(StartAction::OpenSettings) {
            let devices = brp_audio::output_devices().map_err(|e| e.to_string());
            self.settings_dialog
                .open_with(&self.store.settings, devices);
        }
        // `saved` is moved into the dialog-save branch below; keep this before that.
        let saved_any = saved.is_some();
        if let Some(settings) = saved {
            self.store.settings = settings;
            if let Err(error) = self.store.save() {
                self.settings_dialog.error = format!("could not save: {error}");
                self.settings_dialog.open = true;
            } else {
                // The file is now what we wrote, so a stale "not loaded" message no longer holds.
                self.start.error.clear();
                if self.args.nickname.is_none()
                    && let Some(nickname) = &self.store.settings.nickname
                {
                    // The start form, not the saved settings, is what the next open sends.
                    self.start.nickname = nickname.clone();
                }
            }
        }
        if let Some(main) = &self.main
            && (saved_any || output.open_settings)
        {
            main.window.request_redraw();
        }
    }

    fn redraw_popout(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        let Some(key) = self.popouts.key_of(id) else {
            return;
        };
        let (Some(gpu), Some(popout), Some(tiles), Phase::Room(view)) = (
            self.gpu.as_ref(),
            self.popout_windows.get_mut(&id),
            self.tiles.as_mut(),
            &self.phase,
        ) else {
            return;
        };
        view.upload_frames(gpu, tiles);
        let fullscreen = popout.fullscreen;
        let surface = &mut popout.surface;
        let size = surface.size();
        let mut output = UiOutput::default();
        let mut ui_frame = surface.ui.run(&surface.window, [size.0, size.1], |root| {
            output = popout::draw(root, &view.snapshot, &mut self.state, key, fullscreen);
        });
        let placements = pixel_placements(&output, ui_frame.screen.pixels_per_point, size);
        let presented = present(gpu, tiles, surface, &mut ui_frame, &placements);
        let repaint_delay = ui_frame.repaint_delay;
        if repaint_delay.is_zero() {
            if presented {
                surface.window.request_redraw();
            }
        } else {
            self.note_repaint(repaint_delay);
        }

        let had_commands = !output.commands.is_empty();
        let had_window_commands = !output.window_commands.is_empty();
        if let Phase::Room(view) = &mut self.phase
            && had_commands
        {
            view.apply(output.commands, &self.runtime, &self.proxy, &mut self.state);
        }
        if had_window_commands {
            self.apply_window_commands(event_loop, output.window_commands);
        }
        if had_commands || had_window_commands {
            self.request_redraw_all();
        }
    }

    fn apply_window_commands(
        &mut self,
        event_loop: &ActiveEventLoop,
        commands: Vec<WindowCommand>,
    ) {
        for command in commands {
            match command {
                WindowCommand::PopOut(key) => self.open_popout(event_loop, key, false),
                WindowCommand::PopOutFullscreen(key) => self.open_popout(event_loop, key, true),
                WindowCommand::ToggleFullscreen(key) => {
                    if let Some(id) = self.popouts.window_of(key)
                        && let Some(popout) = self.popout_windows.get_mut(&id)
                    {
                        popout.fullscreen = !popout.fullscreen;
                        popout.surface.window.set_fullscreen(
                            popout.fullscreen.then_some(Fullscreen::Borderless(None)),
                        );
                    }
                }
                WindowCommand::ReturnToGrid(key) => {
                    if let Some(id) = self.popouts.remove_key(key) {
                        self.popout_windows.remove(&id);
                    }
                }
            }
        }
    }

    /// Opens a window for `key`. Failures leave the live in the grid and explain why in the
    /// status line; a key already popped out is left where it is.
    fn open_popout(&mut self, event_loop: &ActiveEventLoop, key: TileKey, fullscreen: bool) {
        if self.popouts.is_popped(key) {
            return;
        }
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let title = match &self.phase {
            Phase::Room(view) => live_title(&view.snapshot, key),
            Phase::Start => None,
        }
        .unwrap_or_else(|| "live".to_string());
        let attributes = Window::default_attributes()
            .with_title(format!("brp: {title}"))
            .with_inner_size(DEFAULT_WINDOW_SIZE)
            .with_visible(false);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.state.status = format!("pop-out failed: {error}");
                return;
            }
        };
        let surface = match WindowSurface::new(gpu, window) {
            Ok(surface) => surface,
            Err(error) => {
                self.state.status = format!("pop-out failed: {error}");
                return;
            }
        };
        if fullscreen {
            surface
                .window
                .set_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        // An unsupported format must not flash an empty window, so stay hidden until configured.
        surface.window.set_visible(true);
        let id = surface.window.id();
        self.popouts.insert(id, key);
        surface.window.request_redraw();
        self.popout_windows.insert(
            id,
            PopOutWindow {
                surface,
                fullscreen,
            },
        );
    }

    fn close_popout(&mut self, id: WindowId) {
        self.popouts.remove_window(id);
        self.popout_windows.remove(&id);
        if let Some(main) = &self.main {
            main.window.request_redraw();
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.main.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("brp")
                .with_inner_size(DEFAULT_WINDOW_SIZE),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                tracing::error!(%error, "could not create window");
                event_loop.exit();
                return;
            }
        };
        let (gpu, main) = match GpuContext::new(event_loop, window) {
            Ok(pair) => pair,
            Err(error) => {
                tracing::error!(%error, "could not initialise the GPU");
                event_loop.exit();
                return;
            }
        };
        self.tiles = Some(TileRenderer::new(&gpu.device, gpu.format));
        self.gpu = Some(gpu);
        self.main = Some(main);
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::RoomOpened(Ok(room)) => {
                self.pending_open = None;
                self.state = UiState::new();
                if let Some(main) = &self.main {
                    main.window
                        .set_title(&format!("brp: {}", room.snapshot().nickname));
                }
                self.remember_open(&room);
                self.phase = Phase::Room(Box::new(RoomView::new(room)));
            }
            AppEvent::RoomOpened(Err(message)) => {
                self.pending_open = None;
                self.pending_intent = None;
                self.start.failed(message);
            }
            AppEvent::ShareFinished(outcome) => {
                if let Phase::Room(view) = &mut self.phase {
                    view.pending_share = None;
                }
                self.state.share_pending = false;
                if let Err(message) = outcome {
                    self.state.status = format!("share failed: {message}");
                }
            }
            AppEvent::RoomChanged | AppEvent::NewFrame | AppEvent::Tick => {}
        }
        self.request_redraw_all();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.is_main(id) {
            let Some(main) = self.main.as_mut() else {
                return;
            };
            let response = main.ui.on_window_event(&main.window, &event);
            if response.repaint {
                main.window.request_redraw();
            }
            if response.consumed {
                return;
            }
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::Resized(size) => {
                    if let Some(gpu) = self.gpu.as_ref() {
                        main.resize(gpu, size.width, size.height);
                    }
                }
                WindowEvent::RedrawRequested => self.redraw(event_loop, id),
                _ => {}
            }
            return;
        }
        let Some(popout) = self.popout_windows.get_mut(&id) else {
            return;
        };
        let response = popout
            .surface
            .ui
            .on_window_event(&popout.surface.window, &event);
        if response.repaint {
            popout.surface.window.request_redraw();
        }
        if response.consumed {
            return;
        }
        match event {
            // Closing a pop-out returns its live to the grid; the watch itself continues.
            WindowEvent::CloseRequested => self.close_popout(id),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_ref() {
                    popout.surface.resize(gpu, size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop, id),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.next_repaint {
            Some(deadline) if deadline <= Instant::now() => {
                self.next_repaint = None;
                self.request_redraw_all();
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some(deadline) => event_loop.set_control_flow(ControlFlow::WaitUntil(deadline)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}

/// The egui-point rects of one pass as pixel viewports on a surface of `size`.
fn pixel_placements(
    output: &UiOutput,
    pixels_per_point: f32,
    size: (u32, u32),
) -> Vec<(TileKey, PixelRect)> {
    output
        .tile_rects
        .iter()
        .map(|(key, rect)| (*key, grid::to_pixels(*rect, pixels_per_point, size)))
        .collect()
}

/// Records and submits one window's frame: the placed tiles, then egui on top. A lost surface
/// skips the frame; the next `Resized` reconfigures it. Returns whether the frame was presented,
/// so a caller does not re-request its own redraw for a surface that has nothing to reconfigure
/// it: that would spin until `Resized` arrives.
fn present(
    gpu: &GpuContext,
    tiles: &TileRenderer,
    surface: &mut WindowSurface,
    ui_frame: &mut UiFrame,
    placements: &[(TileKey, PixelRect)],
) -> bool {
    let Some(texture) = surface.acquire() else {
        // The frame's texture deltas must still be applied and freed or they assert on drop.
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let buffers = surface
            .ui
            .prepare(&gpu.device, &gpu.queue, &mut encoder, ui_frame);
        gpu.queue
            .submit(buffers.into_iter().chain(std::iter::once(encoder.finish())));
        surface.ui.cleanup(ui_frame);
        return false;
    };
    let target = texture.texture.create_view(&Default::default());
    tiles.update_fits(&gpu.queue, placements);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let buffers = surface
        .ui
        .prepare(&gpu.device, &gpu.queue, &mut encoder, ui_frame);
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tiles+ui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        tiles.draw(&mut pass, placements);
        surface.ui.paint(&mut pass, ui_frame);
    }
    gpu.queue
        .submit(buffers.into_iter().chain(std::iter::once(encoder.finish())));
    surface.ui.cleanup(ui_frame);
    surface.window.pre_present_notify();
    gpu.queue.present(texture);
    true
}

/// The instant egui wants the next frame, or `None` when it asked for nothing: egui reports
/// `Duration::MAX` in that case, which overflows an `Instant`.
fn repaint_deadline(now: Instant, delay: Duration) -> Option<Instant> {
    now.checked_add(delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finite_delay_becomes_a_deadline_and_no_request_becomes_none() {
        let now = Instant::now();
        assert_eq!(
            repaint_deadline(now, Duration::from_millis(300)),
            Some(now + Duration::from_millis(300))
        );
        assert_eq!(repaint_deadline(now, Duration::ZERO), Some(now));
        assert_eq!(repaint_deadline(now, Duration::MAX), None);
    }
}
