//! The video path against a real GPU: the pipeline has to validate and a leaving tile has to darken.
//! Passes quietly on a machine with no adapter, such as a headless runner without a software driver.

use brp_app::render::{grid::PixelRect, tiles::TileRenderer};

const SIZE: u32 = 64;

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// The middle pixel's brightest channel with a mid-grey frame drawn at `shown`.
fn middle_with(device: &wgpu::Device, queue: &wgpu::Queue, shown: f32) -> u8 {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut tiles = TileRenderer::new(device, format);
    let key = (iroh::SecretKey::from_bytes(&[1; 32]).public(), 1);
    let mut frame = brp_codec::RawFrame::black(16, 16, 0);
    frame.y.fill(180);
    frame.uv.fill(128);
    tiles.upload(device, queue, key, &frame);
    let placements = [(
        key,
        PixelRect {
            x: 0.0,
            y: 0.0,
            width: SIZE as f32,
            height: SIZE as f32,
        },
        shown,
    )];
    tiles.update_fits(queue, &placements);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let stride = (SIZE * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride * SIZE),
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
    let middle = (SIZE / 2 * stride + SIZE / 2 * 4) as usize;
    pixels[middle..middle + 3]
        .iter()
        .copied()
        .max()
        .expect("three channels")
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
