//! wgpu device plus the tile and egui renderers.
pub mod grid;
pub mod tiles;
pub mod ui;
use crate::error::AppError;
use std::sync::Arc;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;
pub struct GpuContext {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
}
impl GpuContext {
    pub fn new(event_loop: &ActiveEventLoop, window: &Arc<Window>) -> Result<Self, AppError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        let surface = instance
            .create_surface(Arc::clone(window))
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
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| AppError::Window("surface is not supported".into()))?;
        config.format =
            egui_wgpu::preferred_framebuffer_format(&surface.get_capabilities(&adapter).formats)
                .map_err(|e| AppError::Window(format!("no usable surface format: {e}")))?;
        surface.configure(&device, &config);
        Ok(Self {
            surface,
            device,
            queue,
            config,
        })
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
    }
}
