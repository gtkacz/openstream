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
                brp_app::ui::own_lives::draw(ui, &snapshot, &mut state, &mut Vec::new());
            },
        );
        shapes = std::mem::take(&mut output.shapes);
        output.textures_delta.clear();
    }
    assert!(shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("encoder could not start"))), "Failure must not be hidden behind Quality & encoding");
}
