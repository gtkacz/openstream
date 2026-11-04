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
    let mut tiles = TileRenderer::new(device, format);
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
    tiles.update_fits(queue, &placements);
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
fn a_tile_darkens_to_the_canvas_as_it_leaves() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let full = middle_with(&device, &queue, 1.0);
    let half = middle_with(&device, &queue, 0.5);
    let none = middle_with(&device, &queue, 0.0);
    assert!(full > 150, "a shown tile was {full}");
    assert!(
        (i32::from(half) - i32::from(full) / 2).abs() <= 2,
        "half shown was {half} of {full}"
    );
    assert_eq!(none, 0, "a tile that has left must match the black canvas");
}
