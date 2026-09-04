use crate::{
    error::AppError,
    render::{GpuContext, ui::EguiLayer, video::VideoRenderer},
};
use brp_codec::RawFrame;
use brp_pipeline::{LatestSlot, ViewerStats};
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow},
    window::{Window, WindowId},
};
pub enum AppEvent {
    NewFrame,
    Status(String),
}
pub struct App {
    title: String,
    description: String,
    slot: Arc<LatestSlot<RawFrame>>,
    stats: Arc<ViewerStats>,
    status: String,
    window: Option<Arc<Window>>,
    gpu: Option<GpuContext>,
    video: Option<VideoRenderer>,
    ui: Option<EguiLayer>,
}
impl App {
    pub fn new(
        title: String,
        description: String,
        slot: Arc<LatestSlot<RawFrame>>,
        stats: Arc<ViewerStats>,
    ) -> Self {
        Self {
            title,
            description,
            slot,
            stats,
            status: "connected, waiting for the first frame".into(),
            window: None,
            gpu: None,
            video: None,
            ui: None,
        }
    }
    fn redraw(&mut self) {
        let (Some(window), Some(gpu), Some(video), Some(ui)) = (
            self.window.as_ref(),
            self.gpu.as_mut(),
            self.video.as_mut(),
            self.ui.as_mut(),
        ) else {
            return;
        };
        if let Some(frame) = self.slot.try_take() {
            video.upload(&gpu.device, &gpu.queue, &frame);
        }
        let surface = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => return,
        };
        let target = surface.texture.create_view(&Default::default());
        let vp = (gpu.config.width, gpu.config.height);
        video.update_fit(&gpu.queue, vp);
        let desc = self.description.clone();
        let status = self.status.clone();
        let f = self
            .stats
            .frames_decoded
            .load(std::sync::atomic::Ordering::Relaxed);
        let ui_frame = ui.run(window, [vp.0, vp.1], |ctx| {
            egui::Window::new("Stats").show(ctx, |ui| {
                ui.monospace(desc);
                ui.monospace(status);
                ui.monospace(format!("decoded {f}"));
            });
        });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        let bufs = ui.prepare(&gpu.device, &gpu.queue, &mut enc, &ui_frame);
        {
            let mut pass = enc
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("video+ui"),
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
            video.draw(&mut pass);
            ui.paint(&mut pass, &ui_frame);
        }
        gpu.queue
            .submit(bufs.into_iter().chain(std::iter::once(enc.finish())));
        ui.cleanup(&ui_frame);
        window.pre_present_notify();
        gpu.queue.present(surface);
    }
}
impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let w = match el.create_window(
            Window::default_attributes()
                .with_title(&self.title)
                .with_inner_size(PhysicalSize::new(1280, 720)),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                tracing::error!(%e,"could not create window");
                el.exit();
                return;
            }
        };
        let gpu = match GpuContext::new(el, &w) {
            Ok(g) => g,
            Err(e) => {
                let _: AppError = e;
                el.exit();
                return;
            }
        };
        self.video = Some(VideoRenderer::new(&gpu.device, gpu.config.format));
        self.ui = Some(EguiLayer::new(&w, &gpu.device, gpu.config.format));
        self.gpu = Some(gpu);
        self.window = Some(w)
    }
    fn user_event(&mut self, _: &ActiveEventLoop, e: AppEvent) {
        if let AppEvent::Status(s) = e {
            self.status = s
        }
        if let Some(w) = &self.window {
            w.request_redraw()
        }
    }
    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, e: WindowEvent) {
        match e {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                if let Some(g) = self.gpu.as_mut() {
                    g.resize(s.width, s.height)
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }
    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        el.set_control_flow(ControlFlow::Wait)
    }
}
