//! The character ramp as a texture, shared by every shader that draws in characters. Each
//! glyph is rasterised once at a large size with a chain of smaller copies, so it stays clean when
//! the shader samples it at the size of one small cell.

use ab_glyph::{Font, FontRef, PxScale};

/// From empty to dense. The first entry is a space, so the darkest cells draw nothing.
pub const RAMP: &str = " .:-=+*#%@";
pub const CELL_WIDTH: u32 = 60;
pub const CELL_HEIGHT: u32 = 100;
const MIPS: u32 = 5;

pub fn atlas(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let glyphs: Vec<char> = RAMP.chars().collect();
    let width = CELL_WIDTH * glyphs.len() as u32;
    let height = CELL_HEIGHT;
    let mut pixels = vec![0u8; (width * height) as usize];
    let font = FontRef::try_from_slice(include_bytes!("../../assets/fonts/IBMPlexMono-Medium.ttf"))
        .expect("the embedded font parses");
    // Plex's advance is 0.6 em, so this size fills the cell's width exactly.
    let scale = PxScale::from(CELL_WIDTH as f32 / 0.6 * 0.94);
    for (index, glyph) in glyphs.iter().enumerate() {
        let id = font.glyph_id(*glyph);
        let left = (index as u32 * CELL_WIDTH) as f32 + CELL_WIDTH as f32 * 0.03;
        let baseline = CELL_HEIGHT as f32 * 0.74;
        let outline =
            font.outline_glyph(id.with_scale_and_position(scale, ab_glyph::point(left, baseline)));
        if let Some(outline) = outline {
            let bounds = outline.px_bounds();
            outline.draw(|x, y, coverage| {
                let px = x as i32 + bounds.min.x as i32;
                let py = y as i32 + bounds.min.y as i32;
                if px >= 0 && py >= 0 && (px as u32) < width && (py as u32) < height {
                    let at = (py as u32 * width + px as u32) as usize;
                    pixels[at] = pixels[at].max((coverage * 255.0) as u8);
                }
            });
        }
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glyph-atlas"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: MIPS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let (mut level_width, mut level_height, mut level) = (width, height, pixels);
    for mip in 0..MIPS {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &level,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(level_width),
                rows_per_image: Some(level_height),
            },
            wgpu::Extent3d {
                width: level_width,
                height: level_height,
                depth_or_array_layers: 1,
            },
        );
        let (next_width, next_height) = ((level_width / 2).max(1), (level_height / 2).max(1));
        let mut next = vec![0u8; (next_width * next_height) as usize];
        for y in 0..next_height {
            for x in 0..next_width {
                let mut sum = 0u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(level_width - 1);
                    let sy = (y * 2 + dy).min(level_height - 1);
                    sum += u32::from(level[(sy * level_width + sx) as usize]);
                }
                next[(y * next_width + x) as usize] = (sum / 4) as u8;
            }
        }
        (level_width, level_height, level) = (next_width, next_height, next);
    }
    texture.create_view(&Default::default())
}

pub fn sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("glyph-sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    })
}
