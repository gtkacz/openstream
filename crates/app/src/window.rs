//! The participant window: a winit loop that draws the tile grid under the egui panels and turns
//! panel commands into room calls.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use brp_proto::SourceKind;
use brp_room::{Room, RoomSnapshot, WatchHandle};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy},
    window::{Window, WindowId},
};

use crate::commands::RoomCommand;
use crate::error::AppError;
use crate::render::grid::{self, PixelRect};
use crate::render::tiles::{TileKey, TileRenderer};
use crate::render::{GpuContext, ui::EguiLayer};
use crate::ui::state::{BitrateMeter, UiState, total_encoded_bytes};
use crate::ui::{self, UiOutput};

/// Wakes the winit event loop for a reason that does not arrive as a `WindowEvent`. Sent through
/// the `EventLoopProxy` from other threads (the room's background tasks, the share task).
pub enum AppEvent {
    /// The room's version counter moved; re-snapshot on the next redraw.
    RoomChanged,
    /// A watched live decoded a frame.
    NewFrame,
    /// Periodic wake so counters refresh while nothing is watched.
    Tick,
    /// The portal picker closed: the live started, or the error to show.
    ShareFinished(Result<(), String>),
}

/// The winit `ApplicationHandler` for the participant window: owns the room handle, the last
/// snapshot, and the GPU and egui state, and turns panel commands into room calls each redraw.
pub struct App {
    runtime: Handle,
    room: Arc<Room>,
    proxy: EventLoopProxy<AppEvent>,
    snapshot: RoomSnapshot,
    ticket: String,
    state: UiState,
    meter: BitrateMeter,
    handles: HashMap<TileKey, WatchHandle>,
    pending_share: Option<JoinHandle<()>>,
    window: Option<Arc<Window>>,
    gpu: Option<GpuContext>,
    tiles: Option<TileRenderer>,
    ui: Option<EguiLayer>,
}

impl App {
    pub fn new(runtime: Handle, room: Arc<Room>, proxy: EventLoopProxy<AppEvent>) -> Self {
        let snapshot = room.snapshot();
        let ticket = room.ticket().to_string();
        Self {
            runtime,
            room,
            proxy,
            snapshot,
            ticket,
            state: UiState::new(),
            meter: BitrateMeter::default(),
            handles: HashMap::new(),
            pending_share: None,
            window: None,
            gpu: None,
            tiles: None,
            ui: None,
        }
    }

    /// A share still waiting on the portal holds an `Arc<Room>`; the caller aborts it before leaving.
    pub fn take_pending_share(&mut self) -> Option<JoinHandle<()>> {
        self.pending_share.take()
    }

    fn refresh(&mut self) {
        if self.room.version() != self.snapshot.version {
            self.snapshot = self.room.snapshot();
        }
        // The relay address can arrive after the first snapshot without bumping the version.
        self.ticket = self.room.ticket().to_string();
        let live: HashSet<TileKey> = self
            .snapshot
            .watches
            .iter()
            .map(|w| (w.publisher, w.live_id))
            .collect();
        self.handles.retain(|key, _| live.contains(key));
        if let Some(tiles) = self.tiles.as_mut() {
            tiles.retain(|key| live.contains(key));
        }
        self.state.upload_kbps = self
            .meter
            .update(total_encoded_bytes(&self.snapshot), Instant::now());
    }

    fn redraw(&mut self) {
        self.refresh();
        let (Some(window), Some(gpu), Some(tiles), Some(ui)) = (
            self.window.as_ref(),
            self.gpu.as_mut(),
            self.tiles.as_mut(),
            self.ui.as_mut(),
        ) else {
            return;
        };
        for (key, handle) in &self.handles {
            if let Some(frame) = handle.slot.try_take() {
                tiles.upload(&gpu.device, &gpu.queue, *key, &frame);
            }
        }
        let surface = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => return,
        };
        let target = surface.texture.create_view(&Default::default());
        let size = (gpu.config.width, gpu.config.height);

        let mut output = UiOutput::default();
        let mut ui_frame = ui.run(window, [size.0, size.1], |root| {
            output = ui::draw(root, &self.snapshot, &self.ticket, &mut self.state);
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
        }

        self.apply(output.commands);
    }

    fn apply(&mut self, commands: Vec<RoomCommand>) {
        if commands.is_empty() {
            return;
        }
        // `Room::watch` spawns its task with `tokio::spawn`, which needs a runtime on this thread.
        // The handle is cloned so the guard does not borrow `self` while `share` needs it mutably.
        let runtime = self.runtime.clone();
        let _guard = runtime.enter();
        self.state.status.clear();
        for command in commands {
            let result = match command {
                RoomCommand::Watch { key, preset_id } => {
                    self.room.watch(key.0, key.1, preset_id).map(|handle| {
                        self.handles.insert(key, handle);
                    })
                }
                RoomCommand::Unwatch(key) => self.room.unwatch(key.0, key.1).map(|()| {
                    self.handles.remove(&key);
                }),
                RoomCommand::StopLive(live_id) => self.room.stop_live(live_id),
                RoomCommand::SetPresets { live_id, presets } => {
                    self.room.set_presets(live_id, presets)
                }
                RoomCommand::Share(kind) => {
                    self.share(kind);
                    Ok(())
                }
            };
            if let Err(error) = result {
                self.state.status = error.to_string();
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn share(&mut self, kind: SourceKind) {
        if self.pending_share.is_some() {
            return;
        }
        let title = self.state.next_title(kind);
        self.state.share_pending = true;
        self.state.status.clear();
        let room = self.room.clone();
        let proxy = self.proxy.clone();
        self.pending_share = Some(self.runtime.spawn(async move {
            let outcome = room
                .start_live(kind, title)
                .await
                .map(|_live_id| ())
                .map_err(|error| error.to_string());
            let _ = proxy.send_event(AppEvent::ShareFinished(outcome));
        }));
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let title = format!("brp: {}", self.snapshot.nickname);
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title(&title)
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
            AppEvent::ShareFinished(outcome) => {
                self.state.share_pending = false;
                self.pending_share = None;
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
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}
