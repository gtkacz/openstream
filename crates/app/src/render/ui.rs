use winit::{event::WindowEvent, window::Window};
pub struct EguiLayer {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
}
pub struct UiFrame {
    pub paint_jobs: Vec<egui::ClippedPrimitive>,
    pub textures_delta: egui::TexturesDelta,
    pub screen: egui_wgpu::ScreenDescriptor,
    /// Zero when egui wants another frame at once, for animations and open popups.
    pub repaint_delay: std::time::Duration,
}
impl EguiLayer {
    pub fn new(window: &Window, device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let renderer = egui_wgpu::Renderer::new(device, format, Default::default());
        Self {
            ctx,
            state,
            renderer,
        }
    }
    pub fn on_window_event(
        &mut self,
        window: &Window,
        event: &WindowEvent,
    ) -> egui_winit::EventResponse {
        self.state.on_window_event(window, event)
    }
    pub fn run(
        &mut self,
        window: &Window,
        size: [u32; 2],
        ui: impl FnMut(&mut egui::Ui),
    ) -> UiFrame {
        let mut vi = egui::ViewportInfo::default();
        egui_winit::update_viewport_info(&mut vi, &self.ctx, window, false);
        let mut input = self.state.take_egui_input(window);
        input.viewports.insert(egui::ViewportId::ROOT, vi);
        let out = self.ctx.run_ui(input, ui);
        self.state
            .handle_platform_output(window, out.platform_output);
        let repaint_delay = out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay)
            .unwrap_or(std::time::Duration::MAX);
        UiFrame {
            paint_jobs: self.ctx.tessellate(out.shapes, out.pixels_per_point),
            textures_delta: out.textures_delta,
            screen: egui_wgpu::ScreenDescriptor {
                size_in_pixels: size,
                pixels_per_point: out.pixels_per_point,
            },
            repaint_delay,
        }
    }
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &mut UiFrame,
    ) -> Vec<wgpu::CommandBuffer> {
        // `drain` empties the delta as it is applied; epaint's `TexturesDelta` panics on drop if
        // it still holds entries, so leaving it merely borrowed (as `&frame...set`) is not enough.
        for (id, deltas) in frame.textures_delta.set.drain() {
            for d in &deltas {
                self.renderer.update_texture(device, queue, id, d);
            }
        }
        self.renderer
            .update_buffers(device, queue, encoder, &frame.paint_jobs, &frame.screen)
    }
    pub fn paint(&self, pass: &mut wgpu::RenderPass<'static>, frame: &UiFrame) {
        self.renderer.render(pass, &frame.paint_jobs, &frame.screen)
    }
    pub fn cleanup(&mut self, frame: &mut UiFrame) {
        for id in frame.textures_delta.free.drain() {
            self.renderer.free_texture(&id);
        }
    }
}
