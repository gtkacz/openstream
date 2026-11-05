//! Draws every watched live as a letterboxed NV12 quad in its own viewport. One pipeline and
//! sampler are shared; each tile owns its planes, its fit uniform, and its bind group.

use std::collections::{HashMap, HashSet};

use brp_codec::RawFrame;
use brp_proto::ChromaLayout;
use bytemuck::{Pod, Zeroable};
use iroh::PublicKey;
use wgpu::util::DeviceExt;

use super::grid::PixelRect;

const SHADER: &str = include_str!("nv12.wgsl");

/// A watched live: publisher and live id. Tiles, watch handles, and per-tile UI choices share it.
pub type TileKey = (PublicKey, u32);

/// Where one tile's video goes this frame, and how shown it is, down to black on its way out.
pub type TilePlacement = (TileKey, PixelRect, f32);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Fit {
    scale: [f32; 2],
    shown: f32,
    flash: f32,
    origin: [f32; 2],
    size: [f32; 2],
    ink: [f32; 4],
    hot: [f32; 4],
    ascii: f32,
    time: f32,
    cell_w: f32,
    cell_h: f32,
    lod: f32,
    count: f32,
    invert: f32,
    _pad: f32,
}

/// What one tile's picture is doing beyond being shown, set by the interface each frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileFx {
    /// How far the picture has reduced to characters.
    pub ascii: f32,
    /// How far the tube has warmed up, from a line to the whole picture.
    pub warm: f32,
}

impl Default for TileFx {
    fn default() -> Self {
        Self {
            ascii: 0.0,
            warm: 1.0,
        }
    }
}

/// A tube switching off: the picture collapses to a bright line, then the line to nothing. Switching
/// on is the same thing backwards.
fn tube(power: f32) -> (f32, f32, f32) {
    const LINE: f32 = 0.006;
    let off = 1.0 - power.clamp(0.0, 1.0);
    if off <= 0.0 {
        (1.0, 1.0, 1.0)
    } else if off < 0.55 {
        let k = off / 0.55;
        (1.0, 1.0 - (1.0 - LINE) * k * k, 1.0 + 3.0 * k)
    } else {
        (1.0 - (off - 0.55) / 0.45, LINE, 4.0)
    }
}

struct Tile {
    width: u32,
    height: u32,
    chroma: ChromaLayout,
    y: wgpu::Texture,
    uv: wgpu::Texture,
    fit: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Sampled from each frame, so a white page reduces to text on dark instead of a block.
    mean_luma: f32,
}

/// Owns the tile pipeline and one GPU-backed tile per watched live, keyed by `TileKey`. The caller
/// uploads decoded frames as they arrive, then calls `update_fits` and `draw` once per redraw with
/// the same placements.
pub struct TileRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    atlas: wgpu::TextureView,
    atlas_sampler: wgpu::Sampler,
    started: std::time::Instant,
    /// A picture fades rather than collapses, and its characters hold still.
    gentle: bool,
    fx: HashMap<TileKey, TileFx>,
    tiles: HashMap<TileKey, Tile>,
    /// Lives already reported as too large to show, so a stream of them logs once.
    refused: HashSet<TileKey>,
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
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
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
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
            atlas: super::glyphs::atlas(device, queue),
            atlas_sampler: super::glyphs::sampler(device),
            started: std::time::Instant::now(),
            gentle: false,
            fx: HashMap::new(),
            tiles: HashMap::new(),
            refused: HashSet::new(),
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
        // The size comes from the sender's stream, and wgpu treats an impossible texture as fatal.
        // Below two pixels wide, the chroma plane would have no width at all.
        let limit = device.limits().max_texture_dimension_2d;
        if frame.width < 2 || frame.height == 0 || frame.width.max(frame.height) > limit {
            if self.refused.insert(key) {
                tracing::warn!(
                    width = frame.width,
                    height = frame.height,
                    limit,
                    "not showing a live this GPU cannot hold"
                );
            }
            return;
        }
        let needs_alloc = self.tiles.get(&key).is_none_or(|t| {
            (t.width, t.height, t.chroma) != (frame.width, frame.height, frame.chroma)
        });
        if needs_alloc {
            let tile = self.allocate(device, frame.width, frame.height, frame.chroma);
            self.tiles.insert(key, tile);
        }
        if let Some(tile) = self.tiles.get_mut(&key) {
            let stride = (frame.y.len() / 251).max(1);
            let samples = frame.y.iter().step_by(stride);
            let (sum, count) = samples.fold((0u64, 0u64), |(sum, count), luma| {
                (sum + u64::from(*luma), count + 1)
            });
            tile.mean_luma = sum as f32 / count.max(1) as f32 / 255.0;
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
                width: frame.chroma_width() as u32,
                height: frame.chroma_rows() as u32,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Drops planes for watches that no longer exist and tiles that have finished leaving.
    pub fn retain(&mut self, keep: impl Fn(&TileKey) -> bool) {
        self.tiles.retain(|key, _| keep(key));
        self.refused.retain(|key| keep(key));
    }

    /// Writes each placed tile's letterbox scale and how shown it is. Call before the render pass is
    /// recorded.
    pub fn update_fits(
        &self,
        queue: &wgpu::Queue,
        placements: &[TilePlacement],
        pixels_per_point: f32,
    ) {
        let rgba = |color: egui::Color32| {
            let [r, g, b, _] = color.to_array();
            [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                1.0,
            ]
        };
        let cell_h = 12.0 * pixels_per_point;
        for (key, rect, shown) in placements {
            if let Some(tile) = self.tiles.get(key) {
                let fx = self.fx.get(key).copied().unwrap_or_default();
                let power = shown.min(fx.warm);
                let (wide, tall, flash) = if self.gentle {
                    (1.0, 1.0, 1.0)
                } else {
                    tube(power)
                };
                let [x, y] = fit_scale(
                    (tile.width, tile.height),
                    (rect.width as u32, rect.height as u32),
                );
                let fit = Fit {
                    scale: [x * wide, y * tall],
                    shown: if self.gentle { power } else { 1. },
                    flash,
                    origin: [rect.x, rect.y],
                    size: [rect.width, rect.height],
                    ink: rgba(crate::ui::theme::accent()),
                    hot: rgba(crate::ui::theme::text()),
                    ascii: fx.ascii,
                    time: if self.gentle {
                        0.0
                    } else {
                        self.started.elapsed().as_secs_f32()
                    },
                    cell_w: cell_h * 0.6,
                    cell_h,
                    lod: (super::glyphs::CELL_HEIGHT as f32 / cell_h).log2().max(0.0),
                    count: super::glyphs::RAMP.chars().count() as f32,
                    invert: if tile.mean_luma > 0.5 { 1.0 } else { 0.0 },
                    _pad: 0.0,
                };
                queue.write_buffer(&tile.fit, 0, bytemuck::bytes_of(&fit));
            }
        }
    }

    /// What the interface wants this tile's picture to do this frame.
    pub fn set_fx(&mut self, key: TileKey, fx: TileFx) {
        self.fx.insert(key, fx);
    }

    /// Passes on that the system asked for less motion.
    pub fn set_gentle(&mut self, gentle: bool) {
        self.gentle = gentle;
    }

    /// Draws every placed tile that has received a frame. Tiles without a frame stay black.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'static>, placements: &[TilePlacement]) {
        pass.set_pipeline(&self.pipeline);
        for (key, rect, _) in placements {
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

    fn allocate(
        &self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        chroma: ChromaLayout,
    ) -> Tile {
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
        // The shader samples in normalised coordinates, so a full-size UV plane needs nothing else.
        let (uv_width, uv_height) = match chroma {
            ChromaLayout::Half => (width / 2, height.div_ceil(2)),
            ChromaLayout::Full => (width, height),
        };
        let uv = make(
            "nv12-uv",
            uv_width,
            uv_height,
            wgpu::TextureFormat::Rg8Unorm,
        );
        let fit = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("nv12-fit"),
            contents: bytemuck::bytes_of(&Fit {
                scale: [1.; 2],
                shown: 1.,
                flash: 1.,
                origin: [0.; 2],
                size: [1.; 2],
                ink: [1.; 4],
                hot: [1.; 4],
                ascii: 0.,
                time: 0.,
                cell_w: 7.,
                cell_h: 12.,
                lod: 0.,
                count: 10.,
                invert: 0.,
                _pad: 0.,
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
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&self.atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.atlas_sampler),
                },
            ],
        });
        Tile {
            width,
            height,
            chroma,
            y,
            uv,
            fit,
            bind_group,
            mean_luma: 0.0,
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
