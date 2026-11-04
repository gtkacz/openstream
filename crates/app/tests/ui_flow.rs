use brp_app::ui::{
    start::{self, StartAction, StartState},
    update::UpdateState,
};

fn text_rect(shapes: &[egui::epaint::ClippedShape], text: &str) -> egui::Rect {
    shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(s) if s.galley.text() == text => {
                Some(s.galley.rect.translate(s.pos.to_vec2()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing visible text: {text}"))
}

#[test]
fn start_screen_names_openstream_and_keeps_the_create_action_clickable_at_narrow_width() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut state = StartState::new("Alex".into());
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(390.0, 844.0),
        )),
        ..Default::default()
    };
    let mut action = None;
    let mut output = ctx.run_ui(input(), |ui| {
        action = start::draw(ui, &mut state, &[], 0, &UpdateState::new(false));
    });
    let shapes = std::mem::take(&mut output.shapes);
    output.textures_delta.clear();
    assert!(text_rect(&shapes, "OpenStream").right() <= 390.0);
    let button = text_rect(&shapes, "Create room");
    assert!(button.left() >= 0.0 && button.right() <= 390.0);
    output.textures_delta.clear();
    for pressed in [true, false] {
        let mut raw = input();
        raw.events = vec![
            egui::Event::PointerMoved(button.center()),
            egui::Event::PointerButton {
                pos: button.center(),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        let mut out = ctx.run_ui(raw, |ui| {
            action = start::draw(ui, &mut state, &[], 0, &UpdateState::new(false));
        });
        out.textures_delta.clear();
    }
    assert_eq!(action, Some(StartAction::Create));
}

#[test]
fn settings_save_is_visible_and_applies_the_draft_in_a_narrow_window() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut dialog = brp_app::ui::settings::SettingsDialog::default();
    dialog.open_with(&brp_app::settings::Settings::default(), Ok(Vec::new()));
    dialog.nickname_text = "  Morgan  ".into();
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(390.0, 844.0),
        )),
        ..Default::default()
    };
    let mut shapes = Vec::new();
    for _ in 0..3 {
        let mut output = ctx.run_ui(input(), |ui| {
            brp_app::ui::settings::draw(ui.ctx(), &mut dialog, false);
        });
        shapes = std::mem::take(&mut output.shapes);
        output.textures_delta.clear();
    }
    let save = text_rect(&shapes, "Save changes");
    assert!(save.left() >= 0.0 && save.right() <= 390.0 && save.bottom() <= 844.0);
    let mut saved = None;
    for pressed in [true, false] {
        let mut raw = input();
        raw.events = vec![
            egui::Event::PointerMoved(save.center()),
            egui::Event::PointerButton {
                pos: save.center(),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        let mut output = ctx.run_ui(raw, |ui| {
            saved = brp_app::ui::settings::draw(ui.ctx(), &mut dialog, false);
        });
        output.textures_delta.clear();
    }
    assert_eq!(
        saved.expect("Save applies the draft").nickname.as_deref(),
        Some("Morgan")
    );
    assert!(!dialog.open);
}

#[test]
fn tab_reveals_stream_controls_without_a_pointer_and_keeps_them_available() {
    use brp_app::ui::{
        state::UiState,
        tiles::{self, Placement},
    };
    use brp_room::{AudioCaptureState, OwnAudioView, RoomSnapshot, WatchState, WatchView};
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let publisher = iroh::SecretKey::from_bytes(&[2; 32]).public();
    let snapshot = RoomSnapshot {
        me: iroh::SecretKey::from_bytes(&[1; 32]).public(),
        nickname: "Alex".into(),
        version: 1,
        members: Vec::new(),
        own_lives: Vec::new(),
        watches: Vec::new(),
        own_audio: OwnAudioView {
            enabled: false,
            state: AudioCaptureState::Off,
            selection: brp_audio::AudioSelection::All,
            subscribers: 0,
            packets_encoded: 0,
        },
        audio_output_error: None,
        master_mute: false,
    };
    let watch = WatchView {
        publisher,
        live_id: 1,
        preset_id: 1,
        state: WatchState::Live,
        frames_decoded: 0,
        keyframe_requests: 0,
        audio: false,
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let mut state = UiState::new();
    let mut shapes = Vec::new();
    for frame in 0..3 {
        let mut raw = egui::RawInput {
            screen_rect: Some(rect),
            time: Some(frame as f64 * 0.1),
            ..Default::default()
        };
        if frame == 1 {
            raw.events.push(egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            });
        }
        let mut output = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                tiles::tile_overlay(
                    ui,
                    &snapshot,
                    &mut state,
                    &mut Vec::new(),
                    &mut Vec::new(),
                    &watch,
                    (publisher, 1),
                    rect,
                    Placement::Grid,
                );
            });
        });
        shapes = std::mem::take(&mut output.shapes);
        output.textures_delta.clear();
    }
    assert!(
        ctx.memory(|memory| memory.focused().is_some()),
        "Tab must enter the toolbar"
    );
    assert!(text_rect(&shapes, "Pop out").is_positive());
}

#[test]
fn an_encoder_failure_stays_visible_with_quality_details_collapsed() {
    use brp_room::{AudioCaptureState, OwnAudioView, OwnLiveView, PresetView, RoomSnapshot};
    let preset = brp_proto::Preset {
        id: 1,
        name: "Source".into(),
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_kbps: 5000,
        codec: brp_proto::Codec::Hevc,
    };
    let snapshot = RoomSnapshot {
        me: iroh::SecretKey::from_bytes(&[1; 32]).public(),
        nickname: "Alex".into(),
        version: 1,
        members: Vec::new(),
        watches: Vec::new(),
        own_lives: vec![OwnLiveView {
            info: brp_proto::LiveInfo {
                id: 1,
                title: "Monitor 1".into(),
                kind: brp_proto::SourceKind::Monitor,
                source_width: 1920,
                source_height: 1080,
                source_fps: 60,
                has_audio: false,
                presets: vec![preset.clone()],
            },
            presets: vec![PresetView {
                preset,
                encoder: None,
                last_error: Some("encoder could not start".into()),
            }],
        }],
        own_audio: OwnAudioView {
            enabled: false,
            state: AudioCaptureState::Off,
            selection: brp_audio::AudioSelection::All,
            subscribers: 0,
            packets_encoded: 0,
        },
        audio_output_error: None,
        master_mute: false,
    };
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut state = brp_app::ui::state::UiState::new();
    let mut shapes = Vec::new();
    for _ in 0..3 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                brp_app::ui::draw(
                    ui,
                    &snapshot,
                    "ticket",
                    &mut state,
                    &std::collections::HashSet::new(),
                    &UpdateState::new(false),
                );
            },
        );
        shapes = std::mem::take(&mut output.shapes);
        output.textures_delta.clear();
    }
    assert!(shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("encoder could not start"))), "Failure must not be hidden behind Quality & encoding");
}

fn window(width: f32, height: f32) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width, height),
        )),
        ..Default::default()
    }
}

fn click_at(mut raw: egui::RawInput, pos: egui::Pos2, pressed: bool) -> egui::RawInput {
    raw.events = vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        },
    ];
    raw
}

/// A room with one remote stream being watched and one stream of your own going out.
fn busy_room() -> brp_room::RoomSnapshot {
    use brp_room::{
        AudioCaptureState, MemberView, OwnAudioView, OwnLiveView, PresetView, RoomSnapshot,
        WatchState, WatchView,
    };
    let info = brp_proto::LiveInfo {
        id: 1,
        title: "Monitor 1".into(),
        kind: brp_proto::SourceKind::Monitor,
        source_width: 2560,
        source_height: 1440,
        source_fps: 60,
        has_audio: false,
        presets: brp_proto::template_presets(2560, 1440, 60, brp_proto::Codec::Hevc),
    };
    let publisher = iroh::SecretKey::from_bytes(&[2; 32]).public();
    RoomSnapshot {
        me: iroh::SecretKey::from_bytes(&[1; 32]).public(),
        nickname: "Alex".into(),
        version: 1,
        members: vec![MemberView {
            id: publisher,
            nickname: "Morgan".into(),
            lives: vec![info.clone()],
            seen_ago_ms: 0,
            path: brp_net::PathKind::Direct,
            has_audio: false,
            gain: 1.0,
        }],
        own_lives: vec![OwnLiveView {
            presets: info
                .presets
                .iter()
                .cloned()
                .map(|preset| PresetView {
                    preset,
                    encoder: None,
                    last_error: None,
                })
                .collect(),
            info,
        }],
        watches: vec![WatchView {
            publisher,
            live_id: 1,
            preset_id: 1,
            state: WatchState::Live,
            frames_decoded: 10,
            keyframe_requests: 0,
            audio: false,
        }],
        own_audio: OwnAudioView {
            enabled: true,
            state: AudioCaptureState::Idle,
            selection: brp_audio::AudioSelection::All,
            subscribers: 0,
            packets_encoded: 0,
        },
        audio_output_error: None,
        master_mute: false,
    }
}

fn run_room(
    ctx: &egui::Context,
    raw: egui::RawInput,
    snapshot: &brp_room::RoomSnapshot,
    state: &mut brp_app::ui::state::UiState,
) -> (brp_app::ui::UiOutput, Vec<egui::epaint::ClippedShape>) {
    let mut result = None;
    let mut output = ctx.run_ui(raw, |ui| {
        result = Some(brp_app::ui::draw(
            ui,
            snapshot,
            "ticket",
            state,
            &std::collections::HashSet::new(),
            &UpdateState::new(false),
        ));
    });
    output.textures_delta.clear();
    (result.expect("the room was drawn"), output.shapes)
}

#[test]
fn the_video_keeps_most_of_the_window_while_sharing_at_every_width() {
    const LARGEST_DOCK: f32 = 128.0;
    for (width, height) in [
        (390.0, 844.0),
        (799.0, 600.0),
        (800.0, 640.0),
        (1000.0, 640.0),
        (1280.0, 800.0),
        (1920.0, 1080.0),
    ] {
        let ctx = egui::Context::default();
        brp_app::ui::theme::install(&ctx);
        let snapshot = busy_room();
        let mut state = brp_app::ui::state::UiState::new();
        let mut placed = None;
        for _ in 0..4 {
            placed = Some(run_room(&ctx, window(width, height), &snapshot, &mut state).0);
        }
        let tile = placed.expect("drawn").tile_rects[0].1;
        assert!(
            tile.height() >= height - LARGEST_DOCK,
            "at {width} wide the tile is only {} of {height} tall",
            tile.height()
        );
        assert!(tile.right() <= width && tile.left() >= 0.0);
    }
}

#[test]
fn a_compact_window_folds_the_room_into_a_sheet_and_keeps_the_video_placed() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let snapshot = busy_room();
    let mut state = brp_app::ui::state::UiState::new();
    let (_, shapes) = run_room(&ctx, window(390.0, 844.0), &snapshot, &mut state);
    let has_text = |shapes: &[egui::epaint::ClippedShape], text: &str| {
        shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(s) if s.galley.text() == text))
    };
    assert!(
        !has_text(&shapes, "Copy link"),
        "the rail is folded away at phone width"
    );
    let room = text_rect(&shapes, "Room").center();
    for pressed in [true, false] {
        let raw = click_at(window(390.0, 844.0), room, pressed);
        run_room(&ctx, raw, &snapshot, &mut state);
    }
    assert!(state.sheet_open);
    let mut last = run_room(&ctx, window(390.0, 844.0), &snapshot, &mut state);
    for _ in 0..30 {
        let mut raw = window(390.0, 844.0);
        raw.predicted_dt = 0.1;
        last = run_room(&ctx, raw, &snapshot, &mut state);
    }
    assert!(has_text(&last.1, "Copy link"), "the sheet holds the rail");
    assert!(
        last.0.tile_rects[0].1.height() > 100.0,
        "the video keeps playing above the sheet"
    );
}

#[test]
fn escape_cancels_settings_but_a_stray_click_on_the_backdrop_keeps_the_draft() {
    use brp_app::{settings::Settings, ui::settings};
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut dialog = settings::SettingsDialog::default();
    dialog.open_with(&Settings::default(), Ok(Vec::new()));
    let run = |raw: egui::RawInput, dialog: &mut settings::SettingsDialog| {
        let mut output = ctx.run_ui(raw, |ui| {
            settings::draw(ui.ctx(), dialog, false);
        });
        output.textures_delta.clear();
    };
    run(window(1280.0, 800.0), &mut dialog);
    for pressed in [true, false] {
        run(
            click_at(window(1280.0, 800.0), egui::pos2(8.0, 8.0), pressed),
            &mut dialog,
        );
    }
    assert!(dialog.open, "the backdrop must not discard a draft");
    let mut escape = window(1280.0, 800.0);
    escape.events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    });
    run(escape, &mut dialog);
    assert!(!dialog.open, "Esc behaves like Cancel");
}

#[test]
fn one_escape_closes_only_the_dialog_on_top() {
    use brp_app::ui::theme;
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let run = |raw: egui::RawInput| {
        let mut dismissed = (false, false);
        let mut output = ctx.run_ui(raw, |ui| {
            dismissed.0 = theme::dialog(ui.ctx(), "under", "Under", |_| ()).1;
            dismissed.1 = theme::dialog(ui.ctx(), "over", "Over", |_| ()).1;
        });
        output.textures_delta.clear();
        dismissed
    };
    run(window(1280.0, 800.0));
    let mut escape = window(1280.0, 800.0);
    escape.events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    });
    assert_eq!(run(escape), (false, true));
}

#[test]
fn a_stalled_stream_does_not_keep_the_window_drawing_flat_out() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut snapshot = busy_room();
    snapshot.watches[0].state = brp_room::WatchState::Reconnecting;
    let mut state = brp_app::ui::state::UiState::new();
    let mut delay = std::time::Duration::ZERO;
    // Past the panel and toolbar settling, which may each ask for one more pass.
    for frame in 0..8 {
        let mut raw = window(1280.0, 800.0);
        raw.time = Some(f64::from(frame) * 0.05);
        let mut output = ctx.run_ui(raw, |ui| {
            brp_app::ui::draw(
                ui,
                &snapshot,
                "ticket",
                &mut state,
                &std::collections::HashSet::new(),
                &UpdateState::new(false),
            );
        });
        output.textures_delta.clear();
        delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    }
    // egui takes one predicted frame off a requested delay so the repaint lands on time; zero is
    // what the window loop reads as "draw again at once".
    assert!(
        delay >= std::time::Duration::from_millis(30),
        "repaint delay is {delay:?}"
    );
}

#[test]
fn dialogs_stay_usable_in_a_window_too_small_for_them() {
    use brp_app::ui::theme;
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut output = ctx.run_ui(window(24.0, 24.0), |ui| {
        theme::dialog(ui.ctx(), "tiny", "Tiny", |_| ());
    });
    output.textures_delta.clear();
}
