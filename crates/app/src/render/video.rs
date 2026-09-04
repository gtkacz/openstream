use brp_codec::RawFrame;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
const SHADER: &str = include_str!("nv12.wgsl");
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Fit {
    scale: [f32; 2],
    _pad: [f32; 2],
}
struct Planes {
    width: u32,
    height: u32,
    y: wgpu::Texture,
    uv: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}
pub struct VideoRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    fit: wgpu::Buffer,
    planes: Option<Planes>,
}
pub fn fit_scale(video: (u32, u32), viewport: (u32, u32)) -> [f32; 2] {
    let va = video.0 as f32 / video.1.max(1) as f32;
    let wa = viewport.0.max(1) as f32 / viewport.1.max(1) as f32;
    if va > wa {
        [1., wa / va]
    } else {
        [va / wa, 1.]
    }
}
impl VideoRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nv12"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nv12-layout"),
            entries: &[
                tex(0),
                tex(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nv12-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nv12-pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nv12-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let fit = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("nv12-fit"),
            contents: bytemuck::bytes_of(&Fit {
                scale: [1.; 2],
                _pad: [0.; 2],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        Self {
            pipeline,
            layout,
            sampler,
            fit,
            planes: None,
        }
    }
    pub fn video_size(&self) -> Option<(u32, u32)> {
        self.planes.as_ref().map(|p| (p.width, p.height))
    }
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &RawFrame) {
        if self
            .planes
            .as_ref()
            .is_none_or(|p| (p.width, p.height) != (frame.width, frame.height))
        {
            self.planes = Some(self.allocate(device, frame.width, frame.height));
        }
        let p = self.planes.as_ref().unwrap();
        let copy = |texture| wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        };
        queue.write_texture(
            copy(&p.y),
            &frame.y,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.y_stride as u32),
                rows_per_image: Some(frame.height),
            },
            wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
        );
        queue.write_texture(
            copy(&p.uv),
            &frame.uv,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.uv_stride as u32),
                rows_per_image: Some(frame.chroma_rows() as u32),
            },
            wgpu::Extent3d {
                width: frame.width / 2,
                height: frame.chroma_rows() as u32,
                depth_or_array_layers: 1,
            },
        );
    }
    pub fn update_fit(&self, queue: &wgpu::Queue, viewport: (u32, u32)) {
        if let Some(size) = self.video_size() {
            queue.write_buffer(
                &self.fit,
                0,
                bytemuck::bytes_of(&Fit {
                    scale: fit_scale(size, viewport),
                    _pad: [0.; 2],
                }),
            );
        }
    }
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'static>) {
        if let Some(p) = &self.planes {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &p.bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
    }
    fn allocate(&self, device: &wgpu::Device, width: u32, height: u32) -> Planes {
        let make = |label, w, h, format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let y = make("nv12-y", width, height, wgpu::TextureFormat::R8Unorm);
        let uv = make(
            "nv12-uv",
            width / 2,
            height.div_ceil(2),
            wgpu::TextureFormat::Rg8Unorm,
        );
        let yv = y.create_view(&Default::default());
        let uvv = uv.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nv12-bind-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&yv),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&uvv),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.fit.as_entire_binding(),
                },
            ],
        });
        Planes {
            width,
            height,
            y,
            uv,
            bind_group,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::fit_scale;
    #[test]
    fn wide_video_in_tall_window_letterboxes() {
        assert_eq!(fit_scale((1920, 1080), (1000, 1000)), [1., 0.5625]);
    }
    #[test]
    fn tall_video_in_wide_window_pillarboxes() {
        let [x, y] = fit_scale((1080, 1920), (1920, 1080));
        assert!((x - 0.3164).abs() < 0.001 && y == 1.0);
    }
    #[test]
    fn matching_aspect_fills_window() {
        assert_eq!(fit_scale((1280, 720), (1920, 1080)), [1., 1.]);
    }
}
