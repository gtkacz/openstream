//! Render the actual native UI without opening a window or connecting to peers.
//! cargo run -p brp --example ui_preview -- /tmp/openstream start 1280 800
//! A fifth argument captures that many 60 Hz frames in, to see motion part way through.
//! A sixth sets the display scale; the width and height stay in physical pixels.
//! Screens: start, recent, invite, focus, room, map, quality, menu, watching, grid, sheet, sharesheet,
//! empty, settings, custom, picker, applications.
//! `PREVIEW_POINTER=x,y` holds the pointer there, to see the light's scramble around it.
//! `PREVIEW_LOST=0.5` reduces a tile to characters by that much, as a lost signal does.
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
    let frames: u32 = args.get(5).map(|s| s.parse()).transpose()?.unwrap_or(60);
    let scale: f32 = args.get(6).map(|s| s.parse()).transpose()?.unwrap_or(1.0);
    let ctx = egui::Context::default();
    ui::theme::install(&ctx);
    ctx.set_zoom_factor(scale);
    let mut start = StartState::new("Alex".into());
    if screen == "invite" {
        start.invite("brproomticketexamplevaluethatrunslong".into());
    }
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
            brp_audio::SendVolumeSupport::Always,
        );
    }
    let mut settings = ui::settings::SettingsDialog::default();
    settings.open_with(&brp_app::settings::Settings::default(), Ok(Vec::new()));
    if screen == "custom" {
        settings.draft.relay =
            brp_app::settings::RelayChoice::Custom("https://relay.example.com/".into());
    }
    let mut snapshot = RoomSnapshot {
        voice: Default::default(),
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
    if matches!(screen, "sheet" | "sharesheet") {
        state.sheet_open = true;
    }
    if screen == "sharesheet" {
        state.sheet = ui::state::CompactSheet::Share;
    }
    // Screens that are a click away: this text is clicked part way through the warm-up.
    let click_on = match screen {
        "menu" => Some("Source"),
        "focus" => Some("Alex"),
        "custom" => Some("Custom"),
        _ => None,
    };
    let mut click_at = None;
    if screen == "quality" {
        state.own_open.insert(1);
        state.quality_open.insert(1);
    }
    let recent = if screen == "recent" {
        vec![brp_app::settings::RecentRoom {
            ticket: "brproomticketexamplevaluethatrunslong".into(),
            last_joined_unix: 0,
        }]
    } else {
        Vec::new()
    };
    if matches!(
        screen,
        "map" | "room" | "quality" | "menu" | "watching" | "grid" | "sheet" | "sharesheet"
    ) {
        snapshot.members.push(MemberView {
            voice: None,
            hearing: true,
            sending: true,
            voice_gain: 1.0,
            signal_lost: false,
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
            rtt_ms: None,
            watching_me: false,
            path: PathKind::Unknown,
            has_audio: false,
            audio_mix: Some(brp_proto::AudioMix::Only {
                names: vec!["firefox".into(), "spotify".into()],
                others: 0,
            }),
            gain: 1.0,
        });
    }
    if screen == "grid" {
        let mut sam = snapshot.members[0].clone();
        sam.id = SecretKey::from_bytes(&[3; 32]).public();
        sam.nickname = "Sam".into();
        sam.path = PathKind::Relayed;
        let mut rae = snapshot.members[0].clone();
        rae.id = SecretKey::from_bytes(&[4; 32]).public();
        rae.nickname = "Rae".into();
        rae.path = PathKind::Direct;
        let others = [sam, rae];
        for (member, state) in others
            .iter()
            .zip([WatchState::Reconnecting, WatchState::Live])
        {
            snapshot.watches.push(WatchView {
                publisher: member.id,
                live_id: 1,
                preset_id: 1,
                state,
                frames_decoded: 5400,
                keyframe_requests: 2,
                audio: false,
            });
        }
        snapshot.members.extend(others);
    }
    if screen == "map" {
        let template = snapshot.members[0].clone();
        snapshot.members[0].path = PathKind::Direct;
        snapshot.members[0].rtt_ms = Some(23);
        snapshot.members[0].watching_me = true;
        let mut rae = template.clone();
        rae.id = SecretKey::from_bytes(&[4; 32]).public();
        rae.nickname = "Rae".into();
        rae.path = PathKind::Relayed;
        rae.rtt_ms = Some(64);
        rae.watching_me = true;
        let mut blender = rae.lives[0].clone();
        blender.id = 2;
        blender.title = "Blender".into();
        rae.lives.push(blender);
        let mut sam = template.clone();
        sam.id = SecretKey::from_bytes(&[3; 32]).public();
        sam.nickname = "Sam".into();
        sam.path = PathKind::Direct;
        sam.rtt_ms = Some(41);
        let mut jo = template.clone();
        jo.id = SecretKey::from_bytes(&[5; 32]).public();
        jo.nickname = "Jo".into();
        jo.lives.clear();
        snapshot.members.extend([rae, sam, jo]);
    }
    if screen == "grid" {
        snapshot.members[0].path = PathKind::Direct;
        snapshot.members[0].rtt_ms = Some(23);
        snapshot.members[1].rtt_ms = Some(64);
        snapshot.members[2].rtt_ms = Some(41);
    }
    if screen == "quality" {
        snapshot.members[0].has_audio = true;
    }
    if matches!(
        screen,
        "map" | "room" | "quality" | "menu" | "sheet" | "sharesheet"
    ) {
        let mut info = snapshot.members[0].lives[0].clone();
        info.title = "Monitor 1".into();
        snapshot.own_lives.push(OwnLiveView {
            full_chroma: true,
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
    if matches!(screen, "watching" | "grid" | "sheet") {
        snapshot.watches.push(WatchView {
            publisher: snapshot.members[0].id,
            live_id: 1,
            preset_id: 1,
            state: if screen == "watching" {
                WatchState::Connecting
            } else {
                WatchState::Live
            },
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
    // The real video path on made-up frames, so overlays are judged against what a picture really
    // does in its tile rather than a colour painted across the tile's whole rect.
    let mut tiles = brp_app::render::tiles::TileRenderer::new(&device, &queue, format);
    let mut backdrop = brp_app::render::backdrop::Backdrop::new(&device, &queue, format);
    let mut bloom = brp_app::render::bloom::Bloom::new(&device, format);
    let lost = std::env::var("PREVIEW_LOST")
        .ok()
        .and_then(|text| text.parse::<f32>().ok());
    for (index, watch) in snapshot
        .watches
        .iter()
        .filter(|w| w.state == WatchState::Live)
        .enumerate()
    {
        let (u, v) = [(150, 110), (160, 140), (100, 150)][index % 3];
        let frame = test_card(u, v);
        tiles.upload(&device, &queue, (watch.publisher, watch.live_id), &frame);
    }
    let mut tile_rects: Vec<(brp_app::render::tiles::TileKey, egui::Rect)> = Vec::new();
    let burn_at = std::env::var("PREVIEW_POINTER").ok().and_then(|text| {
        let (x, y) = text.split_once(',')?;
        Some(egui::pos2(x.parse().ok()?, y.parse().ok()?))
    });
    if burn_at.is_some() {
        ui::motion::set_less_motion(&ctx, false);
    }
    // Warm up egui's multi-pass layout and window placement before capturing.
    for frame in 0..frames {
        let input = egui::RawInput {
            events: if let Some(at) = burn_at {
                vec![egui::Event::PointerMoved(at)]
            } else if matches!(screen, "watching" | "grid") {
                vec![egui::Event::PointerMoved(egui::pos2(
                    width as f32 * 0.7 / scale,
                    height as f32 * 0.3 / scale,
                ))]
            } else if let Some(pos) = click_at.filter(|_| frame == 20 || frame == 21) {
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: frame == 20,
                        modifiers: Default::default(),
                    },
                ]
            } else {
                Vec::new()
            },
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width as f32, height as f32) / scale,
            )),
            focused: true,
            time: Some(f64::from(frame) / 60.0),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            if matches!(
                screen,
                "start" | "recent" | "invite" | "focus" | "settings" | "custom"
            ) {
                ui::start::draw(ui, &mut start, &recent, 7200, &UpdateState::new(false));
                if matches!(screen, "settings" | "custom") {
                    ui::settings::draw(ui.ctx(), &mut settings, false);
                }
            } else {
                let drawn = ui::draw(
                    ui,
                    &snapshot,
                    "preview-ticket",
                    &mut state,
                    &HashSet::new(),
                    &UpdateState::new(false),
                );
                tile_rects = drawn.tile_rects;
                ui::applications::draw(ui.ctx(), &mut state);
            }
        });
        // Tracked until just before the click, since a dialog rises into place over its first frames.
        if let Some(label) = click_on.filter(|_| frame < 20) {
            click_at = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                _ => None,
            });
        }
        for (id, deltas) in output.textures_delta.set.drain() {
            for delta in deltas {
                renderer.update_texture(&device, &queue, id, &delta);
            }
        }
        let output_pixels_per_point = output.pixels_per_point;
        let jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
        let descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: output.pixels_per_point,
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut backdrop_on = false;
        let commands = renderer.update_buffers(&device, &queue, &mut encoder, &jobs, &descriptor);
        {
            let view = texture.create_view(&Default::default());
            let placements: Vec<_> = tile_rects
                .iter()
                .map(|(key, rect)| {
                    let pixels = brp_app::render::grid::to_pixels(
                        *rect,
                        output_pixels_per_point,
                        (width, height),
                    );
                    (*key, pixels, 1.0)
                })
                .collect();
            for (key, _, _) in &placements {
                let fx = ctx.data(|data| data.get_temp(egui::Id::new(("tile-fx", *key))));
                let mut fx: brp_app::render::tiles::TileFx = fx.unwrap_or_default();
                if let Some(lost) = lost {
                    fx.ascii = lost;
                }
                tiles.set_fx(*key, fx);
            }
            tiles.update_fits(&queue, &placements, output_pixels_per_point);
            if let Some(request) = brp_app::render::backdrop::take_request(&ctx) {
                backdrop.skip_opening();
                backdrop.prepare(
                    &device,
                    &queue,
                    &mut encoder,
                    (width, height),
                    output_pixels_per_point,
                    request,
                    Default::default(),
                    true,
                );
                backdrop_on = true;
            }
            if let Some((pass, screen)) = bloom.begin(
                &device,
                &mut encoder,
                (width, height),
                output_pixels_per_point,
            ) {
                let mut pass = pass.forget_lifetime();
                renderer.render(&mut pass, &jobs, &screen);
            }
            bloom.blur(&mut encoder);
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
            let mut pass = pass.forget_lifetime();
            if backdrop_on {
                backdrop.show(&mut pass);
            }
            tiles.draw(&mut pass, &placements);
            renderer.render(&mut pass, &jobs, &descriptor);
            bloom.show(&mut pass);
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

/// A 16:9 frame with an inner panel, so a picture's edges and corners can be told from its tile's.
fn test_card(u: u8, v: u8) -> brp_codec::RawFrame {
    const WIDTH: u32 = 1280;
    const HEIGHT: u32 = 720;
    let mut frame = brp_codec::RawFrame::black(WIDTH, HEIGHT, 0);
    for (row, line) in frame.y.chunks_mut(frame.y_stride).enumerate() {
        for (column, luma) in line.iter_mut().enumerate() {
            let inner = (WIDTH as usize / 5..WIDTH as usize * 4 / 5).contains(&column)
                && (HEIGHT as usize / 5..HEIGHT as usize * 4 / 5).contains(&row);
            *luma = if inner { 170 } else { 90 };
        }
    }
    for pair in frame.uv.as_chunks_mut::<2>().0 {
        *pair = [u, v];
    }
    frame
}
