//! The participant window: a winit loop that shows the start screen until a room is open, then
//! draws the tile grid under the egui panels and hands panel commands to the room view.

use std::sync::Arc;
use std::time::{Duration, Instant};

use brp_room::Room;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy},
    window::{Window, WindowId},
};

use crate::error::AppError;
use crate::launch::{self, Intent, Launch};
use crate::render::grid::{self, PixelRect};
use crate::render::tiles::{TileKey, TileRenderer};
use crate::render::{GpuContext, ui::EguiLayer};
use crate::room_view::RoomView;
use crate::ui::start::{self, StartState};
use crate::ui::state::UiState;
use crate::ui::{self, UiOutput};

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

/// What must be torn down after the loop ends: share tasks still holding room handles, an open
/// that may still be producing a room, and the room itself.
pub struct Shutdown {
    pub room: Option<Arc<Room>>,
    pub tasks: Vec<JoinHandle<()>>,
    /// Awaited, never aborted: a room it produces after the window closed must still be left.
    pub pending_open: Option<JoinHandle<Result<Arc<Room>, String>>>,
}

/// The winit `ApplicationHandler` for the participant window: owns the phase, the window-local UI
/// state, and the GPU and egui state.
pub struct App {
    runtime: Handle,
    proxy: EventLoopProxy<AppEvent>,
    launch: Launch,
    start: StartState,
    phase: Phase,
    state: UiState,
    pending_open: Option<JoinHandle<Result<Arc<Room>, String>>>,
    /// When egui asked for the next frame; `about_to_wait` sleeps until then instead of forever.
    next_repaint: Option<Instant>,
    window: Option<Arc<Window>>,
    gpu: Option<GpuContext>,
    tiles: Option<TileRenderer>,
    ui: Option<EguiLayer>,
}

impl App {
    /// An `intent` from the command line opens the room at once behind the connecting start
    /// screen; `None` waits for the user.
    pub fn new(
        runtime: Handle,
        proxy: EventLoopProxy<AppEvent>,
        launch: Launch,
        nickname: String,
        intent: Option<Intent>,
    ) -> Self {
        let mut app = Self {
            runtime,
            proxy,
            launch,
            start: StartState::new(nickname),
            phase: Phase::Start,
            state: UiState::new(),
            pending_open: None,
            next_repaint: None,
            window: None,
            gpu: None,
            tiles: None,
            ui: None,
        };
        if let Some(intent) = intent {
            app.start.connecting = true;
            app.open(intent);
        }
        app
    }

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

    fn open(&mut self, intent: Intent) {
        let launch = self.launch.clone();
        let nickname = self.start.nickname.clone();
        let room_events = self.proxy.clone();
        let done = self.proxy.clone();
        self.pending_open = Some(self.runtime.spawn(async move {
            let outcome = launch::open_room(&launch, intent, &nickname, room_events)
                .await
                .map_err(|error| error.to_string());
            // The window learns of the outcome through the event; the task output is for the
            // shutdown path, which must leave a room that opened after the window closed.
            let _ = done.send_event(AppEvent::RoomOpened(outcome.clone()));
            outcome
        }));
    }

    fn redraw(&mut self) {
        if let Phase::Room(view) = &mut self.phase {
            view.refresh(&mut self.state, self.tiles.as_mut());
        }
        let (Some(window), Some(gpu), Some(tiles), Some(ui)) = (
            self.window.as_ref(),
            self.gpu.as_mut(),
            self.tiles.as_mut(),
            self.ui.as_mut(),
        ) else {
            return;
        };
        if let Phase::Room(view) = &self.phase {
            view.upload_frames(gpu, tiles);
        }
        let surface = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => return,
        };
        let target = surface.texture.create_view(&Default::default());
        let size = (gpu.config.width, gpu.config.height);

        let mut output = UiOutput::default();
        let mut start_action = None;
        let mut ui_frame = ui.run(window, [size.0, size.1], |root| match &self.phase {
            Phase::Start => start_action = start::draw(root, &mut self.start),
            Phase::Room(view) => {
                output = ui::draw(root, &view.snapshot, &view.ticket, &mut self.state);
            }
        });
        let pixels_per_point = ui_frame.screen.pixels_per_point;
        let placements: Vec<(TileKey, PixelRect)> = output
            .tile_rects
            .iter()
            .map(|(key, rect)| (*key, grid::to_pixels(*rect, pixels_per_point, size)))
            .collect();
        tiles.update_fits(&gpu.queue, &placements);

        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let buffers = ui.prepare(&gpu.device, &gpu.queue, &mut encoder, &mut ui_frame);
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
            tiles.draw(&mut pass, &placements);
            ui.paint(&mut pass, &ui_frame);
        }
        gpu.queue
            .submit(buffers.into_iter().chain(std::iter::once(encoder.finish())));
        ui.cleanup(&mut ui_frame);
        window.pre_present_notify();
        gpu.queue.present(surface);
        if ui_frame.repaint_delay.is_zero() {
            window.request_redraw();
            self.next_repaint = None;
        } else {
            self.next_repaint = repaint_deadline(Instant::now(), ui_frame.repaint_delay);
        }

        let had_commands = !output.commands.is_empty();
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
        if (start_action.is_some() || had_commands)
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("brp")
                .with_inner_size(PhysicalSize::new(1280, 720)),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                tracing::error!(%error, "could not create window");
                event_loop.exit();
                return;
            }
        };
        let gpu = match GpuContext::new(event_loop, &window) {
            Ok(gpu) => gpu,
            Err(error) => {
                let _: AppError = error;
                event_loop.exit();
                return;
            }
        };
        self.tiles = Some(TileRenderer::new(&gpu.device, gpu.config.format));
        self.ui = Some(EguiLayer::new(&window, &gpu.device, gpu.config.format));
        self.gpu = Some(gpu);
        self.window = Some(window);
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::RoomOpened(Ok(room)) => {
                self.pending_open = None;
                self.state = UiState::new();
                if let Some(window) = &self.window {
                    window.set_title(&format!("brp: {}", room.snapshot().nickname));
                }
                self.phase = Phase::Room(Box::new(RoomView::new(room)));
            }
            AppEvent::RoomOpened(Err(message)) => {
                self.pending_open = None;
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
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if let Some(ui) = self.ui.as_mut() {
            let response = ui.on_window_event(&window, &event);
            if response.repaint {
                window.request_redraw();
            }
            if response.consumed {
                return;
            }
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.next_repaint {
            Some(deadline) if deadline <= Instant::now() => {
                self.next_repaint = None;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some(deadline) => event_loop.set_control_flow(ControlFlow::WaitUntil(deadline)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
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
