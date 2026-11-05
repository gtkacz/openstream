//! The video path against a real GPU: the pipeline has to validate and a leaving tile has to darken.
//! Passes quietly on a machine with no adapter, such as a headless runner without a software driver.

use brp_app::render::{grid::PixelRect, tiles::TileRenderer};
use brp_codec::ffmpeg::{FfmpegDecoder, HwDecode, SwsConverter};
use brp_codec::{EncoderConfig, FrameConverter, InputImage, RawFrame, VideoDecoder};
use brp_proto::{ChromaLayout, Codec, PixelFormat, VideoQuality};

const SIZE: u32 = 64;

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// `frame` drawn into a `width` by `height` target at `shown`, as tightly packed RGBA.
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &brp_codec::RawFrame,
    (width, height): (u32, u32),
    shown: f32,
) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut tiles = TileRenderer::new(device, queue, format);
    let key = (iroh::SecretKey::from_bytes(&[1; 32]).public(), 1);
    tiles.upload(device, queue, key, frame);
    let placements = [(
        key,
        PixelRect {
            x: 0.0,
            y: 0.0,
            width: width as f32,
            height: height as f32,
        },
        shown,
    )];
    tiles.update_fits(queue, &placements, 1.0);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let stride = (width * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let view = target.create_view(&Default::default());
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            ..Default::default()
        });
        tiles.draw(&mut pass.forget_lifetime(), &placements);
    }
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).ok();
        });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the GPU finished");
    rx.recv().expect("mapped").expect("mapped");
    let pixels = buffer.slice(..).get_mapped_range().expect("mapped");
    pixels
        .chunks(stride as usize)
        .flat_map(|row| row[..width as usize * 4].iter().copied())
        .collect()
}

/// The middle pixel's brightest channel with a mid-grey frame drawn at `shown`.
fn middle_with(device: &wgpu::Device, queue: &wgpu::Queue, shown: f32) -> u8 {
    let mut frame = brp_codec::RawFrame::black(16, 16, 0);
    frame.y.fill(180);
    frame.uv.fill(128);
    let pixels = render(device, queue, &frame, (SIZE, SIZE), shown);
    let middle = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    pixels[middle..middle + 3]
        .iter()
        .copied()
        .max()
        .expect("three channels")
}

/// What 4:2:0 ruins: one-pixel coloured lines on a dark background, as RGBA. Red and blue alternate
/// on each row, and thin horizontal rules cross them, the way coloured text and UI edges do.
fn fine_colour_detail(width: u32, height: u32) -> Vec<u8> {
    (0..height)
        .flat_map(|row| (0..width).map(move |column| (row, column)))
        .flat_map(|(row, column)| match (row % 8, column % 4) {
            (0, _) => [40, 220, 90, 255],
            (_, 0) => [230, 40, 40, 255],
            (_, 2) => [40, 60, 230, 255],
            _ => [24, 24, 32, 255],
        })
        .collect()
}

/// How far `shown` is from `source`, as a mean per colour channel, out of 255.
fn mean_error(source: &[u8], shown: &[u8]) -> f32 {
    assert_eq!(source.len(), shown.len());
    let total: u64 = source
        .as_chunks::<4>()
        .0
        .iter()
        .zip(shown.as_chunks::<4>().0)
        .flat_map(|(a, b)| a[..3].iter().zip(&b[..3]))
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    total as f32 / (source.len() / 4 * 3) as f32
}

fn converted(source: &[u8], (width, height): (u32, u32), chroma: ChromaLayout) -> RawFrame {
    let mut converter =
        SwsConverter::new(width, height, PixelFormat::Rgba, width, height, chroma).unwrap();
    converter
        .convert(&InputImage {
            width,
            height,
            stride: width as usize * 4,
            format: PixelFormat::Rgba,
            data: source,
            capture_ts_us: 1,
        })
        .unwrap()
        .clone()
}

/// Encodes and decodes `frame` at `quality`, for the picture a viewer would actually get.
fn over_the_codec(frame: &RawFrame, quality: VideoQuality) -> Option<RawFrame> {
    let mut encoder = brp_codec::open_encoder(&EncoderConfig {
        width: frame.width,
        height: frame.height,
        fps: 30,
        bitrate_kbps: 20_000,
        codec: Codec::Hevc,
        quality,
    })
    .ok()?;
    let mut decoder = FfmpegDecoder::open(&encoder.params(), HwDecode::Auto).ok()?;
    let mut last = None;
    for index in 0..8 {
        let frame = RawFrame {
            capture_ts_us: index + 1,
            ..frame.clone()
        };
        for packet in encoder.encode(&frame, index == 0).unwrap() {
            last = decoder.decode(&packet).unwrap().pop().or(last);
        }
    }
    last
}

#[test]
fn full_chroma_reaches_the_screen_with_detail_that_half_chroma_loses() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let size = (64, 64);
    let source = fine_colour_detail(size.0, size.1);
    let shown = |chroma| {
        let frame = converted(&source, size, chroma);
        render(&device, &queue, &frame, size, 1.0)
    };
    let (half, full) = (
        mean_error(&source, &shown(ChromaLayout::Half)),
        mean_error(&source, &shown(ChromaLayout::Full)),
    );
    eprintln!("mean error out of 255: 4:2:0 {half:.2}, 4:4:4 {full:.2}");
    assert!(full < 4.0, "4:4:4 should nearly match its source: {full}");
    assert!(half > full * 3.0, "4:2:0 {half} against 4:4:4 {full}");
}

/// The whole video path short of capture and the network. Gated like the other hardware tests.
#[test]
fn a_viewer_sees_lossless_detail_after_the_real_encoder_and_decoder() {
    if std::env::var_os("BRP_CODEC_TESTS").is_none() {
        return;
    }
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let size = (640, 360);
    let source = fine_colour_detail(size.0, size.1);
    let full = converted(&source, size, ChromaLayout::Full);
    let Some(lossless) = over_the_codec(&full, VideoQuality::Lossless) else {
        eprintln!("no lossless encoder here");
        return;
    };
    assert!(
        lossless.y == full.y && lossless.uv == full.uv,
        "not lossless"
    );
    let chroma = over_the_codec(&full, VideoQuality::Chroma444).expect("4:4:4 encodes");
    let half = converted(&source, size, ChromaLayout::Half);
    let standard = over_the_codec(&half, VideoQuality::Standard).expect("4:2:0 encodes");
    let error = |frame: &RawFrame| mean_error(&source, &render(&device, &queue, frame, size, 1.0));
    let (lossless, chroma, standard) = (error(&lossless), error(&chroma), error(&standard));
    eprintln!(
        "mean error out of 255: 4:2:0 {standard:.2}, 4:4:4 {chroma:.2}, lossless {lossless:.2}"
    );
    assert!(lossless < 4.0, "{lossless}");
    assert!(chroma < standard, "4:4:4 {chroma} against 4:2:0 {standard}");
    assert!(
        standard > lossless * 3.0,
        "4:2:0 {standard} against {lossless}"
    );
}

#[test]
fn a_leaving_tile_collapses_towards_a_bright_line_and_then_to_nothing() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let full = middle_with(&device, &queue, 1.0);
    let collapsing = middle_with(&device, &queue, 0.5);
    let none = middle_with(&device, &queue, 0.0);
    assert!(full > 150, "a shown tile was {full}");
    assert!(
        collapsing > full,
        "collapsing it brightens {full} to {collapsing}"
    );
    assert_eq!(none, 0, "a tile that has left must match the black canvas");
}

#[test]
fn a_frame_the_gpu_cannot_hold_is_skipped_rather_than_fatal() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut tiles = TileRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let key = (iroh::SecretKey::from_bytes(&[1; 32]).public(), 1);
    let too_large = device.limits().max_texture_dimension_2d + 2;
    // A one-pixel-wide frame would ask for a chroma plane with no width at all.
    for (width, height) in [(too_large, 2), (2, too_large), (1, 2)] {
        tiles.upload(
            &device,
            &queue,
            key,
            &brp_codec::RawFrame::black(width, height, 0),
        );
    }
    tiles.upload(&device, &queue, key, &brp_codec::RawFrame::black(16, 16, 0));
}

/// A render target and the bytes read back from it, for the passes that draw the whole window.
fn draw_to_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    (width, height): (u32, u32),
    format: wgpu::TextureFormat,
    draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView),
) -> Vec<u8> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let stride = (width * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    let view = target.create_view(&Default::default());
    draw(&mut encoder, &view);
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).ok();
        });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the GPU finished");
    rx.recv().expect("mapped").expect("mapped");
    let pixels = buffer.slice(..).get_mapped_range().expect("mapped");
    pixels
        .chunks(stride as usize)
        .flat_map(|row| row[..width as usize * 4].iter().copied())
        .collect()
}

/// The channels the phosphor is strongest and weakest in, so a check holds for any tube's colour.
fn phosphor_channels() -> (usize, usize) {
    let accent = brp_app::ui::theme::accent().to_array();
    let by_strength = |pick: fn(u8, u8) -> bool| {
        (0..3).fold(0, |best, channel| {
            if pick(accent[channel], accent[best]) {
                channel
            } else {
                best
            }
        })
    };
    (by_strength(|a, b| a > b), by_strength(|a, b| a < b))
}

fn mean_channel(pixels: &[u8], width: u32, channel: usize, rows: std::ops::Range<u32>) -> f32 {
    let (mut sum, mut count) = (0u64, 0u64);
    for row in rows {
        let line = &pixels[(row * width * 4) as usize..((row + 1) * width * 4) as usize];
        for pixel in line.chunks(4) {
            sum += u64::from(pixel[channel]);
            count += 1;
        }
    }
    sum as f32 / count as f32
}

fn draw_the_horizon(device: &wgpu::Device, queue: &wgpu::Queue, size: (u32, u32)) -> Vec<u8> {
    use brp_app::render::backdrop::{Backdrop, BackdropKind, BackdropRequest};
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut backdrop = Backdrop::new(device, queue, format);
    backdrop.skip_opening();
    draw_to_pixels(device, queue, size, format, |encoder, view| {
        backdrop.prepare(
            device,
            queue,
            encoder,
            size,
            1.0,
            BackdropRequest {
                kind: BackdropKind::Horizon,
                shown: 1.0,
                signal: 1.0,
            },
            Default::default(),
            false,
        );
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            ..Default::default()
        });
        backdrop.show(&mut pass.forget_lifetime());
    })
}

#[test]
fn the_signal_gathers_in_patches_of_the_light_and_the_blues_still_lead() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    const WIDTH: usize = 640;
    let pixels = draw_the_horizon(&device, &queue, (WIDTH as u32, 360));
    // Every blue in the palette has more blue than green, and only the signal does not.
    let (mut lit, mut bands) = (0, [0usize; 8]);
    for (index, pixel) in pixels.chunks(4).enumerate() {
        lit += usize::from(pixel[1].max(pixel[2]) > 96);
        if pixel[1] > 96 && pixel[1] > pixel[2] + 24 {
            bands[(index % WIDTH) * bands.len() / WIDTH] += 1;
        }
    }
    let signal: usize = bands.iter().sum();
    assert!(signal * 20 > lit, "more than a trace: {signal} of {lit}");
    assert!(signal * 2 < lit, "the blues lead: {signal} of {lit}");
    let (most, least) = (bands.iter().max(), bands.iter().min());
    assert!(
        most.unwrap() / 4 > *least.unwrap(),
        "gathered, not scattered: {bands:?}"
    );
}

#[test]
fn the_horizon_lights_the_foot_of_the_window_and_leaves_the_top_dark() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let size = (640, 360);
    let pixels = draw_the_horizon(&device, &queue, size);
    let (lit, _) = phosphor_channels();
    let (top, foot) = (
        mean_channel(&pixels, size.0, lit, 0..40),
        mean_channel(&pixels, size.0, lit, 320..360),
    );
    assert!(foot > top + 30.0, "foot {foot} against top {top}");
}

/// Every window draws this from its first frame, so a shader a platform's compiler rejects would stop
/// the app at launch. With nothing drawn to glow, it has to leave the window as it found it.
#[test]
fn the_chromes_glow_runs_on_this_gpu_and_adds_nothing_when_there_is_nothing_to_glow() {
    use brp_app::render::bloom::Bloom;
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let size = (320, 180);
    let mut bloom = Bloom::new(&device, format);
    let pixels = draw_to_pixels(&device, &queue, size, format, |encoder, view| {
        drop(
            bloom
                .begin(&device, encoder, size, 1.0)
                .expect("a place to draw"),
        );
        bloom.blur(encoder);
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.25,
                        g: 0.25,
                        b: 0.25,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            ..Default::default()
        });
        bloom.show(&mut pass.forget_lifetime());
    });
    assert!(
        pixels
            .chunks(4)
            .all(|pixel| pixel[0] == 64 && pixel[3] == 255)
    );
}

#[test]
fn a_tile_that_lost_its_signal_reduces_to_characters_in_the_phosphors_colour() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let size = (320, 180);
    let key = (iroh::SecretKey::from_bytes(&[1; 32]).public(), 1);
    let mut tiles = TileRenderer::new(&device, &queue, format);
    let mut frame = RawFrame::black(320, 180, 0);
    frame.y.fill(160);
    tiles.upload(&device, &queue, key, &frame);
    tiles.set_fx(
        key,
        brp_app::render::tiles::TileFx {
            ascii: 1.0,
            warm: 1.0,
        },
    );
    let placements = [(
        key,
        PixelRect {
            x: 0.0,
            y: 0.0,
            width: size.0 as f32,
            height: size.1 as f32,
        },
        1.0,
    )];
    tiles.update_fits(&queue, &placements, 1.0);
    let pixels = draw_to_pixels(&device, &queue, size, format, |encoder, view| {
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            ..Default::default()
        });
        tiles.draw(&mut pass.forget_lifetime(), &placements);
    });
    let (lit, unlit) = phosphor_channels();
    let (mut dark, mut inked, mut tinted) = (0, 0, 0);
    for pixel in pixels.chunks(4) {
        if pixel[lit] < 16 {
            dark += 1;
        } else if pixel[lit] > 48 {
            inked += 1;
            tinted += usize::from(pixel[lit] > pixel[unlit] + 24);
        }
    }
    assert!(dark > 1000, "a grey picture must break up: {dark} dark");
    assert!(inked > 1000, "into characters: {inked} inked");
    assert!(
        tinted * 10 > inked * 9,
        "of the phosphor: {tinted} of {inked}"
    );
}
