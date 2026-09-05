//! Draws every watched live as a letterboxed NV12 quad in its own viewport. One pipeline and
//! sampler are shared; each tile owns its planes, its fit uniform, and its bind group.

use std::collections::HashMap;

use brp_codec::RawFrame;
use bytemuck::{Pod, Zeroable};
use iroh::PublicKey;
use wgpu::util::DeviceExt;

use super::grid::PixelRect;

const SHADER: &str = include_str!("nv12.wgsl");

/// A watched live: publisher and live id. Tiles, watch handles, and per-tile UI choices share it.
pub type TileKey = (PublicKey, u32);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Fit {
    scale: [f32; 2],
    _pad: [f32; 2],
}

struct Tile {
    width: u32,
    height: u32,
    y: wgpu::Texture,
    uv: wgpu::Texture,
    fit: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

pub struct TileRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    tiles: HashMap<TileKey, Tile>,
}

/// Clip-space scale that letterboxes or pillarboxes `video` inside `viewport`.
pub fn fit_scale(video: (u32, u32), viewport: (u32, u32)) -> [f32; 2] {
    let va = video.0 as f32 / video.1.max(1) as f32;
    let wa = viewport.0.max(1) as f32 / viewport.1.max(1) as f32;
    if va > wa {
        [1., wa / va]
    } else {
        [va / wa, 1.]
    }
}

impl TileRenderer {
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nv12-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nv12-pipeline"),
            layout: Some(&pipeline_layout),
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
        Self {
            pipeline,
            layout,
            sampler,
            tiles: HashMap::new(),
        }
    }

    /// Uploads a decoded frame, reallocating the tile's planes when the size changes.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: TileKey,
        frame: &RawFrame,
    ) {
        let needs_alloc = self
            .tiles
            .get(&key)
            .is_none_or(|t| (t.width, t.height) != (frame.width, frame.height));
        if needs_alloc {
            let tile = self.allocate(device, frame.width, frame.height);
            self.tiles.insert(key, tile);
        }
        let Some(tile) = self.tiles.get(&key) else {
            return;
        };
        let copy = |texture| wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        };
        queue.write_texture(
            copy(&tile.y),
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
            copy(&tile.uv),
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

    /// Drops planes for watches that no longer exist.
    pub fn retain(&mut self, keep: impl Fn(&TileKey) -> bool) {
        self.tiles.retain(|key, _| keep(key));
    }

    /// Writes each placed tile's letterbox scale. Call before the render pass is recorded.
    pub fn update_fits(&self, queue: &wgpu::Queue, placements: &[(TileKey, PixelRect)]) {
        for (key, rect) in placements {
            if let Some(tile) = self.tiles.get(key) {
                let fit = Fit {
                    scale: fit_scale(
                        (tile.width, tile.height),
                        (rect.width as u32, rect.height as u32),
                    ),
                    _pad: [0.; 2],
                };
                queue.write_buffer(&tile.fit, 0, bytemuck::bytes_of(&fit));
            }
        }
    }

    /// Draws every placed tile that has received a frame. Tiles without a frame stay black.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'static>, placements: &[(TileKey, PixelRect)]) {
        pass.set_pipeline(&self.pipeline);
        for (key, rect) in placements {
            let Some(tile) = self.tiles.get(key) else {
                continue;
            };
            if rect.width < 1.0 || rect.height < 1.0 {
                continue;
            }
            pass.set_viewport(rect.x, rect.y, rect.width, rect.height, 0.0, 1.0);
            pass.set_bind_group(0, &tile.bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
    }

    fn allocate(&self, device: &wgpu::Device, width: u32, height: u32) -> Tile {
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
        let fit = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("nv12-fit"),
            contents: bytemuck::bytes_of(&Fit {
                scale: [1.; 2],
                _pad: [0.; 2],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let y_view = y.create_view(&Default::default());
        let uv_view = uv.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nv12-bind-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&y_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&uv_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: fit.as_entire_binding(),
                },
            ],
        });
        Tile {
            width,
            height,
            y,
            uv,
            fit,
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
