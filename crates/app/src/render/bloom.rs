//! The soft halo around bright text and lines. The interface is drawn again at a quarter of the size,
//! blurred, and screened over everything. One pipeline serves every window of the same format.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::passes::{self, Target};

const SHADER: &str = include_str!("bloom.wgsl");
const STRENGTH: f32 = 0.3;
const DIVISOR: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Blur {
    step: [f32; 2],
    strength: f32,
    pad: f32,
}

struct Sized {
    size: (u32, u32),
    small: Target,
    spare: Target,
    across: wgpu::BindGroup,
    down: wgpu::BindGroup,
    screen: wgpu::BindGroup,
}

pub struct Bloom {
    format: wgpu::TextureFormat,
    blur_pipeline: wgpu::RenderPipeline,
    screen_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    sized: Option<Sized>,
}

impl Bloom {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom-layout"),
            entries: &[
                passes::uniform_entry(0),
                passes::texture_entry(1),
                passes::sampler_entry(2),
            ],
        });
        // Screen rather than add: adding pushes amber towards yellow once red has run out.
        let screen = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::OneMinusDst,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        Self {
            format,
            blur_pipeline: passes::pipeline(device, &shader, &layout, "fs_blur", format, None),
            screen_pipeline: passes::pipeline(
                device,
                &shader,
                &layout,
                "fs_screen",
                format,
                Some(screen),
            ),
            layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("bloom-sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            sized: None,
        }
    }

    fn fit(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if self.sized.as_ref().is_some_and(|sized| sized.size == size) {
            return;
        }
        let small = passes::target(
            device,
            self.format,
            size.0 / DIVISOR,
            size.1 / DIVISOR,
            "bloom-small",
        );
        let spare = passes::target(
            device,
            self.format,
            size.0 / DIVISOR,
            size.1 / DIVISOR,
            "bloom-spare",
        );
        let group = |step: [f32; 2], strength: f32, source: &wgpu::TextureView| {
            let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("bloom-blur"),
                contents: bytemuck::bytes_of(&Blur {
                    step,
                    strength,
                    pad: 0.0,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom-group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            })
        };
        let across = group([1.0 / small.width as f32, 0.0], 1.0, &small.view);
        let down = group([0.0, 1.0 / small.height as f32], 1.0, &spare.view);
        let screen = group([0.0, 0.0], STRENGTH, &small.view);
        self.sized = Some(Sized {
            size,
            small,
            spare,
            across,
            down,
            screen,
        });
    }

    /// Where the interface gets drawn small, and at what scale. Call `blur` after drawing to it.
    pub fn begin<'a>(
        &mut self,
        device: &wgpu::Device,
        encoder: &'a mut wgpu::CommandEncoder,
        size: (u32, u32),
        pixels_per_point: f32,
    ) -> Option<(wgpu::RenderPass<'a>, egui_wgpu::ScreenDescriptor)> {
        self.fit(device, size);
        let sized = self.sized.as_ref()?;
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [sized.small.width, sized.small.height],
            pixels_per_point: pixels_per_point * sized.small.width as f32 / size.0.max(1) as f32,
        };
        Some((
            passes::begin(encoder, &sized.small.view, "bloom-interface"),
            screen,
        ))
    }

    pub fn blur(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(sized) = &self.sized else {
            return;
        };
        for (target, group, label) in [
            (&sized.spare, &sized.across, "bloom-across"),
            (&sized.small, &sized.down, "bloom-down"),
        ] {
            let mut pass = passes::begin(encoder, &target.view, label);
            pass.set_pipeline(&self.blur_pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// Screens the halo over everything, last thing in the window's pass.
    pub fn show(&self, pass: &mut wgpu::RenderPass<'static>) {
        if let Some(sized) = &self.sized {
            pass.set_pipeline(&self.screen_pipeline);
            pass.set_bind_group(0, &sized.screen, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
