//! wgpu device plus the tile and egui renderers. `GpuContext` is the half every window shares;
//! `surface::WindowSurface` is the half each window owns.
pub mod grid;
pub mod surface;
pub mod tiles;
pub mod ui;

use std::sync::Arc;

use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::error::AppError;
use surface::WindowSurface;

/// The GPU state shared by every window: one instance, adapter, device, and queue, and the
/// surface format every window is configured with so one tile pipeline serves them all.
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub format: wgpu::TextureFormat,
}

impl GpuContext {
    /// Picks the adapter against the main window's surface and returns that surface configured.
    /// The main window's surface is created here, not by [`WindowSurface::new`], because wgpu
    /// needs it to choose a compatible adapter and a window has at most one surface.
    pub fn new(
        event_loop: &ActiveEventLoop,
        window: Arc<Window>,
    ) -> Result<(Self, WindowSurface), AppError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        let surface = instance
            .create_surface(Arc::clone(&window))
            .map_err(|e| AppError::Window(format!("create_surface: {e}")))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .map_err(|e| AppError::Window(format!("no suitable GPU adapter: {e}")))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("brp"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| AppError::Window(format!("request_device: {e}")))?;
        let format =
            egui_wgpu::preferred_framebuffer_format(&surface.get_capabilities(&adapter).formats)
                .map_err(|e| AppError::Window(format!("no usable surface format: {e}")))?;
        let gpu = Self {
            instance,
            adapter,
            device,
            queue,
            format,
        };
        let main = WindowSurface::configure(&gpu, window, surface)?;
        Ok((gpu, main))
    }
}
