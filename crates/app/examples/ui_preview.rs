//! Render the actual native UI without opening a window or connecting to peers.
//! cargo run -p brp --example ui_preview -- /tmp/openstream start 1280 800
//! Screens: start, room, watching, empty, settings, picker, applications.
use std::{collections::HashSet, error::Error, io::Write};

use brp_app::ui::{self, start::StartState, state::UiState, update::UpdateState};
use brp_audio::AudioSelection;
use brp_net::PathKind;
use brp_proto::{Codec, LiveInfo, SourceKind, template_presets};
use brp_room::{
    AudioCaptureState, EncoderView, MemberView, OwnAudioView, OwnLiveView, PresetView,
    RoomSnapshot, WatchState, WatchView,
};
use iroh::SecretKey;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    let prefix = args.get(1).map(String::as_str).unwrap_or("/tmp/openstream");
    let screen = args.get(2).map(String::as_str).unwrap_or("start");
    let width: u32 = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(1280);
    let height: u32 = args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(800);
    let ctx = egui::Context::default();
    ui::theme::install(&ctx);
    let mut start = StartState::new("Alex".into());
    let mut state = UiState::new();
    if screen == "picker" {
        state.open_picker(
            SourceKind::Monitor,
            vec![brp_capture::SourceDescriptor {
                id: brp_capture::SourceId(1),
                kind: SourceKind::Monitor,
                name: "Built-in display".into(),
                width: 2560,
                height: 1440,
            }],
        );
    }
    if screen == "applications" {
        state.open_applications(
            vec![brp_audio::AudioSource {
                key: brp_audio::AppKey::new("firefox"),
                label: "Firefox".into(),
            }],
            &brp_app::settings::AudioApplications::default(),
        );
    }
    let mut settings = ui::settings::SettingsDialog::default();
    settings.open_with(&brp_app::settings::Settings::default(), Ok(Vec::new()));
    let mut snapshot = RoomSnapshot {
        me: SecretKey::from_bytes(&[1; 32]).public(),
        nickname: "Alex".into(),
        version: 1,
        members: Vec::new(),
        own_lives: Vec::new(),
        watches: Vec::new(),
        own_audio: OwnAudioView {
            enabled: true,
            state: AudioCaptureState::Idle,
            selection: AudioSelection::All,
            subscribers: 0,
            packets_encoded: 0,
        },
        audio_output_error: None,
        master_mute: false,
    };
    if screen == "room" || screen == "watching" {
        snapshot.members.push(MemberView {
            id: SecretKey::from_bytes(&[2; 32]).public(),
            nickname: "Morgan".into(),
            lives: vec![LiveInfo {
                id: 1,
                title: "Monitor 1".into(),
                kind: SourceKind::Monitor,
                source_width: 2560,
                source_height: 1440,
                source_fps: 60,
                has_audio: false,
                presets: template_presets(2560, 1440, 60, Codec::Hevc),
            }],
            seen_ago_ms: 0,
            path: PathKind::Unknown,
            has_audio: false,
            gain: 1.0,
        });
    }
    if screen == "room" {
        let mut info = snapshot.members[0].lives[0].clone();
        info.title = "Monitor 1".into();
        snapshot.own_lives.push(OwnLiveView {
            presets: info
                .presets
                .iter()
                .cloned()
                .map(|preset| PresetView {
                    preset,
                    encoder: Some(EncoderView {
                        name: "hevc_vaapi",
                        subscribers: 1,
                        frames_encoded: 1800,
                        bytes_encoded: 8_000_000,
                        dropped_at_input: 0,
                    }),
                    last_error: None,
                })
                .collect(),
            info,
        });
    }
    if screen == "watching" {
        snapshot.watches.push(WatchView {
            publisher: snapshot.members[0].id,
            live_id: 1,
            preset_id: 1,
            state: WatchState::Connecting,
            frames_decoded: 0,
            keyframe_requests: 0,
            audio: false,
        });
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = egui_wgpu::Renderer::new(&device, format, Default::default());
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("UI preview"),
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
    // Warm up egui's multi-pass layout and window placement before capturing.
    for frame in 0..3 {
        let input = egui::RawInput {
            events: if screen == "watching" {
                vec![egui::Event::PointerMoved(egui::pos2(
                    width as f32 * 0.7,
                    height as f32 * 0.3,
                ))]
            } else {
                Vec::new()
            },
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width as f32, height as f32),
            )),
            time: Some(frame as f64),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            if screen == "start" || screen == "settings" {
                ui::start::draw(ui, &mut start, &[], 0, &UpdateState::new(false));
                if screen == "settings" {
                    ui::settings::draw(ui.ctx(), &mut settings, false);
                }
            } else {
                ui::draw(
                    ui,
                    &snapshot,
                    "preview-ticket",
                    &mut state,
                    &HashSet::new(),
                    &UpdateState::new(false),
                );
                ui::applications::draw(ui.ctx(), &mut state);
            }
        });
        for (id, deltas) in output.textures_delta.set.drain() {
            for delta in deltas {
                renderer.update_texture(&device, &queue, id, &delta);
            }
        }
        let jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
        let descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: output.pixels_per_point,
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        let commands = renderer.update_buffers(&device, &queue, &mut encoder, &jobs, &descriptor);
        {
            let view = texture.create_view(&Default::default());
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
            renderer.render(&mut pass.forget_lifetime(), &jobs, &descriptor);
        }
        queue.submit(commands.into_iter().chain([encoder.finish()]));
        for id in output.textures_delta.free.drain() {
            renderer.free_texture(&id);
        }
    }
    let stride = (width * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).ok();
        });
    device.poll(wgpu::PollType::wait_indefinitely())?;
    rx.recv()??;
    let pixels = buffer.slice(..).get_mapped_range()?;
    let path = format!("{prefix}-{screen}-{width}.ppm");
    let mut file = std::fs::File::create(&path)?;
    write!(file, "P6\n{width} {height}\n255\n")?;
    for row in pixels.chunks(stride as usize) {
        for pixel in row[..width as usize * 4].as_chunks::<4>().0 {
            file.write_all(&pixel[..3])?;
        }
    }
    println!("{path}");
    Ok(())
}
