//! The half of the GPU state each window owns: its surface, the surface configuration, and its
//! egui context. Every surface uses the shared format so the one tile pipeline draws into any.

use std::sync::Arc;

use winit::window::Window;

use super::GpuContext;
use super::ui::EguiLayer;
use crate::error::AppError;

pub struct WindowSurface {
    pub window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pub ui: EguiLayer,
}

impl WindowSurface {
    /// A surface for a window other than the main one, configured with the shared format.
    /// Refused when this surface cannot take that format, so the caller keeps the live in the
    /// grid and tells the user rather than building a second pipeline.
    pub fn new(gpu: &GpuContext, window: Arc<Window>) -> Result<Self, AppError> {
        let surface = gpu
            .instance
            .create_surface(Arc::clone(&window))
            .map_err(|e| AppError::Window(format!("create_surface: {e}")))?;
        Self::configure(gpu, window, surface)
    }

    pub(super) fn configure(
        gpu: &GpuContext,
        window: Arc<Window>,
        surface: wgpu::Surface<'static>,
    ) -> Result<Self, AppError> {
        let formats = surface.get_capabilities(&gpu.adapter).formats;
        if !formats.contains(&gpu.format) {
            return Err(AppError::Window(format!(
                "surface does not support {:?} (offers {formats:?})",
                gpu.format
            )));
        }
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| AppError::Window("surface is not supported".into()))?;
        config.format = gpu.format;
        surface.configure(&gpu.device, &config);
        let ui = EguiLayer::new(&window, &gpu.device, gpu.format);
        Ok(Self {
            window,
            surface,
            config,
            ui,
        })
    }

    pub fn resize(&mut self, gpu: &GpuContext, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&gpu.device, &self.config);
        }
    }

    /// Current surface size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// The texture to draw this frame into, or `None` when the surface is lost or outdated; the
    /// next `Resized` reconfigures it, so a skipped frame is the right response.
    pub fn acquire(&self) -> Option<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
            _ => None,
        }
    }
}
