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
    // Long enough for the card to finish growing into the rail.
    for frame in 0..60 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(f64::from(frame) / 60.0),
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
    // One row of controls, its separator, and the gaps around the tile, at every width.
    const LARGEST_DOCK: f32 = 74.0;
    const WINDOW_EDGES: f32 = 16.0;
    const RAIL: f32 = 288.0;
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
        // Long enough for a new tile to finish growing into its place.
        for frame in 0..60 {
            let raw = timed(window(width, height), frame);
            placed = Some(run_room(&ctx, raw, &snapshot, &mut state).0);
        }
        let tile = placed.expect("drawn").tile_rects[0].1;
        // The tile has its picture's shape, so one direction fills and the other is whatever is left.
        let rail = if width < 800.0 { 0.0 } else { RAIL };
        assert!(
            tile.height() >= height - LARGEST_DOCK || tile.width() >= width - rail - WINDOW_EDGES,
            "at {width} by {height} the tile is only {:?}",
            tile.size()
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
            dismissed.0 = theme::dialog(ui.ctx(), "under", "Under", true, |_| ())
                .is_some_and(|(_, dismissed)| dismissed);
            dismissed.1 = theme::dialog(ui.ctx(), "over", "Over", true, |_| ())
                .is_some_and(|(_, dismissed)| dismissed);
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
    // Past every spring settling, which legitimately asks for frames while it moves.
    for frame in 0..120 {
        let raw = timed(window(1280.0, 800.0), frame);
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
        theme::dialog(ui.ctx(), "tiny", "Tiny", true, |_| ());
    });
    output.textures_delta.clear();
}

fn timed(mut raw: egui::RawInput, frame: u32) -> egui::RawInput {
    raw.time = Some(f64::from(frame) / 60.0);
    raw
}

#[test]
fn a_button_centres_its_icon_on_its_label() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut output = ctx.run_ui(window(400.0, 200.0), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            brp_app::ui::theme::action(ui, brp_app::ui::theme::icons::LINK, "Copy link", true);
        });
    });
    let shapes = std::mem::take(&mut output.shapes);
    output.textures_delta.clear();
    let label = text_rect(&shapes, "Copy link");
    let icon = text_rect(&shapes, brp_app::ui::theme::icons::LINK);
    assert!(
        (label.center().y - icon.center().y).abs() <= 1.0,
        "label at {} and icon at {}",
        label.center().y,
        icon.center().y
    );
}

#[test]
fn tiles_flow_to_their_new_places_and_track_a_resized_window_directly() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut snapshot = busy_room();
    let mut state = brp_app::ui::state::UiState::new();
    let mut frame = 0;
    let mut run =
        |snapshot: &brp_room::RoomSnapshot, state: &mut brp_app::ui::state::UiState, width: f32| {
            frame += 1;
            // Short enough that two wide pictures sit side by side.
            run_room(&ctx, timed(window(width, 600.0), frame), snapshot, state)
                .0
                .tile_rects
        };
    let mut alone = Vec::new();
    for _ in 0..60 {
        alone = run(&snapshot, &mut state, 1280.0);
    }
    let settled = alone[0].1;

    // A second stream starts: the first tile must head for its half of the grid, not teleport.
    let mut second = snapshot.watches[0].clone();
    second.live_id = 2;
    snapshot.watches.push(second);
    let moving = run(&snapshot, &mut state, 1280.0)[0].1;
    assert!(
        moving.width() > settled.width() * 0.6,
        "a tile must not jump on the frame the grid changes, it is {} of {}",
        moving.width(),
        settled.width()
    );
    let mut halves = Vec::new();
    for _ in 0..60 {
        halves = run(&snapshot, &mut state, 1280.0);
    }
    assert_eq!(halves.len(), 2);
    assert!(halves[0].1.width() < settled.width() * 0.6);
    assert!(halves[0].1.right() <= halves[1].1.left());

    // Resizing is direct manipulation: the same pass must already be at the new size.
    let before = halves[0].1;
    let resized = run(&snapshot, &mut state, 1600.0)[0].1;
    assert!(
        resized.width() > before.width() + 100.0,
        "the tile lagged behind the window: {} then {}",
        before.width(),
        resized.width()
    );
}

#[test]
fn a_closing_dialog_leaves_gradually_without_blocking_the_room_behind_it() {
    use brp_app::{settings::Settings, ui::settings};
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut dialog = settings::SettingsDialog::default();
    dialog.open_with(&Settings::default(), Ok(Vec::new()));
    let mut frame = 0;
    let mut clicked_behind = false;
    let mut pass = |raw: egui::RawInput,
                    dialog: &mut settings::SettingsDialog,
                    clicked_behind: &mut bool| {
        frame += 1;
        let mut output = ctx.run_ui(timed(raw, frame), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                *clicked_behind |= ui
                    .put(
                        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 100.0)),
                        egui::Button::new("behind"),
                    )
                    .clicked();
            });
            settings::draw(ui.ctx(), dialog, false);
        });
        output.textures_delta.clear();
        output.shapes
    };
    for _ in 0..60 {
        pass(window(1280.0, 800.0), &mut dialog, &mut clicked_behind);
    }
    dialog.open = false;
    let has_title = |shapes: &[egui::epaint::ClippedShape]| {
        shapes.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::Text(s) if s.galley.text() == "Settings"),
        )
    };
    let leaving = pass(window(1280.0, 800.0), &mut dialog, &mut clicked_behind);
    assert!(has_title(&leaving), "it must not vanish on the first frame");
    for pressed in [true, false] {
        pass(
            click_at(window(1280.0, 800.0), egui::pos2(50.0, 50.0), pressed),
            &mut dialog,
            &mut clicked_behind,
        );
    }
    assert!(
        clicked_behind,
        "the room takes input while the dialog leaves"
    );
    let mut gone = Vec::new();
    for _ in 0..60 {
        gone = pass(window(1280.0, 800.0), &mut dialog, &mut clicked_behind);
    }
    assert!(!has_title(&gone), "and then it is gone");
}

#[test]
fn a_dropdown_still_highlights_the_row_under_the_pointer() {
    use brp_app::ui::theme;
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut choice = 1;
    let mut frame = 0;
    let mut pass = |raw: egui::RawInput, choice: &mut i32| {
        frame += 1;
        let mut output = ctx.run_ui(timed(raw, frame), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                theme::dropdown(ui, "row-test", "One", None, |ui| {
                    ui.selectable_value(choice, 1, "First");
                    ui.selectable_value(choice, 2, "Second");
                });
            });
        });
        output.textures_delta.clear();
        output.shapes
    };
    let closed = pass(window(400.0, 400.0), &mut choice);
    let button = text_rect(&closed, "One").center();
    for pressed in [true, false] {
        pass(click_at(window(400.0, 400.0), button, pressed), &mut choice);
    }
    let open = pass(window(400.0, 400.0), &mut choice);
    let row = text_rect(&open, "Second").center();
    let mut hover = window(400.0, 400.0);
    hover.events = vec![egui::Event::PointerMoved(row)];
    pass(hover, &mut choice);
    // Long enough for the popup to finish fading in, since its opacity scales the row's fill.
    let mut hovered = Vec::new();
    for _ in 0..30 {
        hovered = pass(window(400.0, 400.0), &mut choice);
    }
    let hover_fill = ctx.global_style().visuals.widgets.hovered.weak_bg_fill;
    assert!(
        hovered
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Rect(r) if r.fill == hover_fill)),
        "the wrapper's transparent surfaces must not leak into the popup"
    );
}

/// A room you have just joined: one member sharing a stream nobody here watches yet.
fn joined_room() -> brp_room::RoomSnapshot {
    let mut snapshot = busy_room();
    snapshot.own_lives.clear();
    snapshot.watches.clear();
    snapshot.members[0].path = brp_net::PathKind::Unknown;
    snapshot
}

#[test]
fn at_the_default_window_size_a_joined_room_lists_the_member_s_stream_and_its_switch() {
    // DEFAULT_WINDOW_SIZE, in points as macOS now opens it.
    let (width, height) = (1280.0, 720.0);
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let snapshot = joined_room();
    let mut state = brp_app::ui::state::UiState::new();
    let has_text = |shapes: &[egui::epaint::ClippedShape], text: &str| {
        shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(s) if s.galley.text() == text))
    };
    let mut last = Vec::new();
    for frame in 0..120 {
        last = run_room(
            &ctx,
            timed(window(width, height), frame),
            &snapshot,
            &mut state,
        )
        .1;
    }
    assert!(has_text(&last, "Morgan"), "the member is listed");
    assert!(
        has_text(&last, "Monitor 1"),
        "the member's stream is listed"
    );
    assert!(has_text(&last, "Watch"), "the stream can be switched on");
}

/// Draws the room for long enough that anything arriving settles, starting at `first_frame`
/// because egui's clock must only ever move forward.
fn settled_room_from(
    first_frame: u32,
    ctx: &egui::Context,
    size: (f32, f32),
    snapshot: &brp_room::RoomSnapshot,
    state: &mut brp_app::ui::state::UiState,
) -> (brp_app::ui::UiOutput, Vec<egui::epaint::ClippedShape>) {
    let mut last = None;
    for frame in first_frame..first_frame + 60 {
        last = Some(run_room(
            ctx,
            timed(window(size.0, size.1), frame),
            snapshot,
            state,
        ));
    }
    last.expect("drawn")
}

fn settled_room(
    ctx: &egui::Context,
    size: (f32, f32),
    snapshot: &brp_room::RoomSnapshot,
    state: &mut brp_app::ui::state::UiState,
) -> (brp_app::ui::UiOutput, Vec<egui::epaint::ClippedShape>) {
    settled_room_from(0, ctx, size, snapshot, state)
}

fn text_rects(shapes: &[egui::epaint::ClippedShape], text: &str) -> Vec<egui::Rect> {
    shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(s) if s.galley.text() == text => {
                Some(s.galley.rect.translate(s.pos.to_vec2()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn the_rail_keeps_its_width_whatever_its_contents_ask_for() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let snapshot = busy_room();
    let mut state = brp_app::ui::state::UiState::new();
    // Quality details hold the rail's widest rows.
    state.quality_open.insert(1);
    let (output, shapes) = settled_room(&ctx, (1280.0, 900.0), &snapshot, &mut state);
    let tile = output.tile_rects[0].1;
    assert!(
        tile.left() <= 296.5,
        "the rail pushed the video to {} instead of leaving it at 296",
        tile.left()
    );
    for label in ["Copy link", "Copy ticket", "Quality & encoding"] {
        assert!(
            text_rect(&shapes, label).right() <= 280.0,
            "{label} overflows"
        );
    }
}

#[test]
fn a_persons_name_and_detail_are_centred_on_their_avatar() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let snapshot = busy_room();
    let mut state = brp_app::ui::state::UiState::new();
    let (_, shapes) = settled_room(&ctx, (1280.0, 800.0), &snapshot, &mut state);
    for (initial, name, detail) in [("A", "Alex", "You"), ("M", "Morgan", "Direct")] {
        let avatar = text_rect(&shapes, initial);
        let block = text_rect(&shapes, name).union(text_rect(&shapes, detail));
        assert!(
            (avatar.center().y - block.center().y).abs() <= 0.5,
            "{name}: avatar at {} and text at {}",
            avatar.center().y,
            block.center().y
        );
    }
}

#[test]
fn a_rail_card_keeps_its_title_detail_and_controls_on_one_left_edge() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let snapshot = busy_room();
    let mut state = brp_app::ui::state::UiState::new();
    let (_, shapes) = settled_room(&ctx, (1280.0, 800.0), &snapshot, &mut state);
    let edge = text_rect(&shapes, "Monitor 1").left();
    let detail = text_rects(&shapes, "2560 × 1440 · 60 fps")[0].left();
    let disclosure = text_rect(&shapes, "Quality & encoding").left();
    assert!(
        (detail - edge).abs() < 0.5,
        "detail at {detail}, title at {edge}"
    );
    assert!(
        (disclosure - edge).abs() < 0.5,
        "disclosure at {disclosure}, title at {edge}"
    );
}

#[test]
fn the_form_stays_clickable_beside_the_wall() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut state = start::StartState::new("Alex".into());
    let mut action = None;
    let mut shapes = Vec::new();
    for frame in 0..3 {
        let mut output = ctx.run_ui(timed(window(1280.0, 800.0), frame), |ui| {
            start::draw(ui, &mut state, &[], 0, &UpdateState::new(false));
        });
        shapes = std::mem::take(&mut output.shapes);
        output.textures_delta.clear();
    }
    let button = text_rect(&shapes, "Create room").center();
    for (frame, pressed) in [(3, true), (4, false)] {
        let raw = timed(click_at(window(1280.0, 800.0), button, pressed), frame);
        let mut output = ctx.run_ui(raw, |ui| {
            action = start::draw(ui, &mut state, &[], 0, &UpdateState::new(false));
        });
        output.textures_delta.clear();
    }
    assert_eq!(action, Some(StartAction::Create));
}

#[test]
fn the_start_form_is_centred_in_the_body() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut state = start::StartState::new("Alex".into());
    let mut shapes = Vec::new();
    for frame in 0..3 {
        let mut output = ctx.run_ui(timed(window(1280.0, 800.0), frame), |ui| {
            start::draw(ui, &mut state, &[], 0, &UpdateState::new(false));
        });
        shapes = std::mem::take(&mut output.shapes);
        output.textures_delta.clear();
    }
    let title = text_rect(&shapes, "Share your screen");
    let join_label = text_rect(&shapes, "Join room");
    // The Join button is 40 tall around its label, so its bottom edge is 20 below the label's centre.
    let form = (title.top() + join_label.center().y + 20.0) / 2.0;
    let body = (24.0 + 36.0 + 8.0 + 800.0 - 24.0) / 2.0;
    assert!(
        (form - body).abs() <= 1.0,
        "form centred at {form}, body at {body}"
    );
}

#[test]
fn the_start_screen_shares_one_column_on_a_very_wide_window() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut state = start::StartState::new("Alex".into());
    let mut output = ctx.run_ui(window(2560.0, 1440.0), |ui| {
        start::draw(ui, &mut state, &[], 0, &UpdateState::new(false));
    });
    let shapes = std::mem::take(&mut output.shapes);
    output.textures_delta.clear();
    // 1200 wide, centred in 2560: the brand and the form both start at 680.
    let title = text_rect(&shapes, "Share your screen");
    assert!(
        (title.left() - 680.0).abs() <= 1.0,
        "form starts at {}",
        title.left()
    );
}

#[test]
fn a_custom_frame_rate_keeps_its_value_until_one_is_typed() {
    use brp_app::{settings::Settings, ui::settings};
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut dialog = settings::SettingsDialog::default();
    dialog.open_with(&Settings::default(), Ok(Vec::new()));
    let saved_fps = dialog.draft.fps;
    let mut frame = 0;
    let mut pass = |raw: egui::RawInput, dialog: &mut settings::SettingsDialog| {
        frame += 1;
        let mut output = ctx.run_ui(timed(raw, frame), |ui| {
            settings::draw(ui.ctx(), dialog, false);
        });
        output.textures_delta.clear();
        output.shapes
    };
    let mut shapes = Vec::new();
    for _ in 0..30 {
        shapes = pass(window(1280.0, 800.0), &mut dialog);
    }
    let custom = text_rect(&shapes, "Custom").center();
    for pressed in [true, false] {
        pass(
            click_at(window(1280.0, 800.0), custom, pressed),
            &mut dialog,
        );
    }
    assert!(dialog.custom_fps);
    assert_eq!(
        dialog.draft.fps, saved_fps,
        "choosing Custom changes nothing yet"
    );
    for _ in 0..30 {
        shapes = pass(window(1280.0, 800.0), &mut dialog);
    }
    let thirty = text_rect(&shapes, "30 fps").center();
    for pressed in [true, false] {
        pass(
            click_at(window(1280.0, 800.0), thirty, pressed),
            &mut dialog,
        );
    }
    assert_eq!(dialog.draft.fps, 30);
    assert!(!dialog.custom_fps);
}

#[test]
fn a_radio_row_and_a_check_row_take_a_click_anywhere_along_their_label() {
    use brp_app::ui::theme;
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let (mut choice, mut checked) = (1, false);
    let mut frame = 0;
    let mut pass = |raw: egui::RawInput, choice: &mut i32, checked: &mut bool| {
        frame += 1;
        let mut output = ctx.run_ui(timed(raw, frame), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                theme::radio(ui, choice, 1, "First choice");
                theme::radio(ui, choice, 2, "Second choice");
                theme::check(ui, checked, "An option");
            });
        });
        output.textures_delta.clear();
        output.shapes
    };
    let shapes = pass(window(400.0, 400.0), &mut choice, &mut checked);
    for label in ["Second choice", "An option"] {
        let at = text_rect(&shapes, label).center();
        for pressed in [true, false] {
            pass(
                click_at(window(400.0, 400.0), at, pressed),
                &mut choice,
                &mut checked,
            );
        }
    }
    assert_eq!(choice, 2);
    assert!(checked);
}

#[test]
fn unfolded_quality_details_show_each_preset_and_fold_away_again() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let snapshot = busy_room();
    let mut state = brp_app::ui::state::UiState::new();
    let (_, shut) = settled_room(&ctx, (1280.0, 1000.0), &snapshot, &mut state);
    assert!(text_rects(&shut, "480p").is_empty());
    let header = text_rect(&shut, "Quality & encoding").center();
    for (frame, pressed) in [(60, true), (61, false)] {
        let click = timed(click_at(window(1280.0, 1000.0), header, pressed), frame);
        run_room(&ctx, click, &snapshot, &mut state);
    }
    assert!(state.quality_open.contains(&1));
    let (_, open) = settled_room_from(62, &ctx, (1280.0, 1000.0), &snapshot, &mut state);
    assert!(!text_rects(&open, "480p").is_empty());
}

#[test]
fn a_toolbar_too_wide_for_its_tile_does_not_keep_the_window_drawing_flat_out() {
    use brp_app::ui::{
        state::UiState,
        tiles::{self, Placement},
    };
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let snapshot = busy_room();
    let watch = snapshot.watches[0].clone();
    let key = (watch.publisher, watch.live_id);
    // Far narrower than the buttons in its toolbar, with the pointer holding the toolbar up.
    let tile = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(120.0, 120.0));
    let mut state = UiState::new();
    let mut delay = std::time::Duration::ZERO;
    for frame in 0..120 {
        let mut raw = timed(window(800.0, 600.0), frame);
        // Once only: any pointer input asks for a frame of its own, which would hide what is tested.
        if frame == 0 {
            raw.events = vec![egui::Event::PointerMoved(tile.center())];
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
                    key,
                    tile,
                    Placement::Grid,
                );
            });
        });
        output.textures_delta.clear();
        delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    }
    assert!(!delay.is_zero(), "the window never got to sleep");
}

#[test]
fn a_long_label_on_a_radio_or_check_stops_at_the_edge_with_an_ellipsis() {
    use brp_app::ui::theme;
    const LONG: &str = "An application with a name far too long for the row it has to sit in";
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let (mut choice, mut checked) = (0, false);
    let mut output = ctx.run_ui(window(200.0, 200.0), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            theme::radio(ui, &mut choice, 1, LONG);
            theme::check(ui, &mut checked, LONG);
        });
    });
    output.textures_delta.clear();
    let mut seen = 0;
    for shape in &output.shapes {
        if let egui::Shape::Text(text) = &shape.shape
            && text.galley.text().starts_with("An application")
        {
            seen += 1;
            let right = text.galley.rect.translate(text.pos.to_vec2()).right();
            assert!(right <= 200.0, "the label runs out to {right}");
            assert!(text.galley.elided, "a cut label has to say that it was cut");
        }
    }
    assert_eq!(seen, 2);
}

#[test]
fn checks_in_a_wrapped_row_move_to_the_next_line_rather_than_truncating() {
    use brp_app::ui::theme;
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut checked = false;
    let mut output = ctx.run_ui(window(240.0, 200.0), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for label in ["First option", "Second option", "Third option"] {
                    theme::check(ui, &mut checked, label);
                }
            });
        });
    });
    let shapes = std::mem::take(&mut output.shapes);
    output.textures_delta.clear();
    let (first, third) = (
        text_rect(&shapes, "First option"),
        text_rect(&shapes, "Third option"),
    );
    assert!(third.top() > first.bottom(), "the row never wrapped");
    assert!(third.right() <= 240.0);
}

fn accent_surfaces(shapes: &[egui::epaint::ClippedShape]) -> usize {
    // The logo is drawn in the accent too, in bars far thinner than any control.
    const CONTROL_AT_LEAST: f32 = 24.0;
    fn count(shape: &egui::Shape) -> usize {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().map(count).sum(),
            egui::Shape::Rect(rect)
                if rect.fill == brp_app::ui::theme::ACCENT
                    && rect.rect.height() >= CONTROL_AT_LEAST =>
            {
                1
            }
            _ => 0,
        }
    }
    shapes.iter().map(|shape| count(&shape.shape)).sum()
}

fn lonely_room() -> brp_room::RoomSnapshot {
    let mut snapshot = busy_room();
    snapshot.members.clear();
    snapshot.own_lives.clear();
    snapshot.watches.clear();
    snapshot
}

fn room_with_an_unwatched_stream() -> brp_room::RoomSnapshot {
    let mut snapshot = busy_room();
    snapshot.own_lives.clear();
    snapshot.watches.clear();
    snapshot
}

fn click_text(
    ctx: &egui::Context,
    first_frame: u32,
    size: (f32, f32),
    at: egui::Pos2,
    snapshot: &brp_room::RoomSnapshot,
    state: &mut brp_app::ui::state::UiState,
) -> Vec<brp_app::commands::RoomCommand> {
    let mut commands = Vec::new();
    for (offset, pressed) in [(0, true), (1, false)] {
        let raw = timed(
            click_at(window(size.0, size.1), at, pressed),
            first_frame + offset,
        );
        commands.extend(run_room(ctx, raw, snapshot, state).0.commands);
    }
    commands
}

#[test]
fn a_tile_has_the_shape_of_its_picture_with_its_name_on_the_picture() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let snapshot = busy_room();
    let mut state = brp_app::ui::state::UiState::new();
    let (output, shapes) = settled_room(&ctx, (1440.0, 900.0), &snapshot, &mut state);
    let tile = output.tile_rects[0].1;
    let preset = &snapshot.members[0].lives[0].presets[0];
    let picture = preset.width as f32 / preset.height as f32;
    assert!(
        (tile.width() / tile.height() - picture).abs() < 0.01,
        "{tile:?} is not the shape of a {picture} picture"
    );
    let chip = text_rect(&shapes, "Morgan · Monitor 1");
    assert!(tile.contains_rect(chip));
    assert!(
        chip.top() - tile.top() < 24.0,
        "the chip floats {} below the picture's top",
        chip.top() - tile.top()
    );
}

#[test]
fn a_closed_tile_fades_where_it_stood_and_its_video_is_released_once_it_has_gone() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut snapshot = busy_room();
    let mut second = snapshot.watches[0].clone();
    second.live_id = 2;
    snapshot.watches.push(second);
    let closing = (snapshot.watches[1].publisher, snapshot.watches[1].live_id);
    let mut state = brp_app::ui::state::UiState::new();
    let (settled, _) = settled_room(&ctx, (1440.0, 900.0), &snapshot, &mut state);
    let stood = settled.tile_rects[1].1;
    assert!(
        state.keeps_picture(&closing),
        "its planes must outlive its watch"
    );

    snapshot.watches.pop();
    let first = run_room(
        &ctx,
        timed(window(1440.0, 900.0), 60),
        &snapshot,
        &mut state,
    )
    .0;
    assert_eq!(first.tile_rects.len(), 1);
    let (key, rect, shown) = first.leaving_tiles[0];
    assert_eq!(key, closing);
    assert!(shown > 0.8 && shown < 1.0, "started at {shown}");
    assert!(
        (rect.center() - stood.center()).length() < 1.0,
        "it left its place: {rect:?} from {stood:?}"
    );

    for frame in 61..180 {
        run_room(
            &ctx,
            timed(window(1440.0, 900.0), frame),
            &snapshot,
            &mut state,
        );
    }
    assert_eq!(state.gone_tiles, vec![closing]);
    assert!(!state.keeps_picture(&closing));
}

#[test]
fn a_tile_watched_again_mid_exit_stops_leaving_and_carries_on_from_where_it_was() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut snapshot = busy_room();
    let mut state = brp_app::ui::state::UiState::new();
    settled_room(&ctx, (1440.0, 900.0), &snapshot, &mut state);
    let watch = snapshot.watches.pop().expect("one watch");
    let leaving = run_room(
        &ctx,
        timed(window(1440.0, 900.0), 60),
        &snapshot,
        &mut state,
    )
    .0;
    assert_eq!(leaving.leaving_tiles.len(), 1);
    snapshot.watches.push(watch);
    let back = run_room(
        &ctx,
        timed(window(1440.0, 900.0), 61),
        &snapshot,
        &mut state,
    )
    .0;
    assert!(back.leaving_tiles.is_empty() && state.gone_tiles.is_empty());
    assert_eq!(back.tile_rects.len(), 1);
}

#[test]
fn a_tile_popped_out_goes_to_its_window_without_an_exit_in_the_grid() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let snapshot = busy_room();
    let key = (snapshot.watches[0].publisher, snapshot.watches[0].live_id);
    let mut state = brp_app::ui::state::UiState::new();
    settled_room(&ctx, (1440.0, 900.0), &snapshot, &mut state);
    let popped = std::collections::HashSet::from([key]);
    let mut result = None;
    let mut output = ctx.run_ui(timed(window(1440.0, 900.0), 60), |ui| {
        result = Some(brp_app::ui::draw(
            ui,
            &snapshot,
            "ticket",
            &mut state,
            &popped,
            &UpdateState::new(false),
        ));
    });
    output.textures_delta.clear();
    let drawn = result.expect("drawn");
    assert!(drawn.tile_rects.is_empty() && drawn.leaving_tiles.is_empty());
}

#[test]
fn alone_in_a_room_the_canvas_holds_the_invite_and_the_one_accent_share_button() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let snapshot = lonely_room();
    let mut state = brp_app::ui::state::UiState::new();
    let (_, shapes) = settled_room(&ctx, (1440.0, 900.0), &snapshot, &mut state);
    assert!(!text_rects(&shapes, "You’re the first here").is_empty());
    assert_eq!(
        text_rects(&shapes, "You’re the first here").len(),
        1,
        "said once, on the canvas rather than also in the rail"
    );
    assert_eq!(accent_surfaces(&shapes), 1, "one primary action on screen");
    // Two labelled Share buttons: the canvas's accent one and the dock's neutral one.
    let shares = text_rects(&shapes, "Share monitor");
    assert_eq!(shares.len(), 2);
    let on_canvas = shares
        .iter()
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .expect("two");
    let commands = click_text(
        &ctx,
        60,
        (1440.0, 900.0),
        on_canvas.center(),
        &snapshot,
        &mut state,
    );
    assert!(commands.iter().any(|command| matches!(
        command,
        brp_app::commands::RoomCommand::Share {
            kind: brp_proto::SourceKind::Monitor,
            source: None
        }
    )));
}

#[test]
fn a_stream_on_the_canvas_starts_with_one_click_and_its_tile_grows_out_of_that_card() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut snapshot = room_with_an_unwatched_stream();
    let mut state = brp_app::ui::state::UiState::new();
    let (_, shapes) = settled_room(&ctx, (1440.0, 900.0), &snapshot, &mut state);
    let card = text_rect(&shapes, "Morgan · Monitor 1");
    let commands = click_text(
        &ctx,
        60,
        (1440.0, 900.0),
        card.center(),
        &snapshot,
        &mut state,
    );
    let preset_id = commands
        .iter()
        .find_map(|command| match command {
            brp_app::commands::RoomCommand::Watch { preset_id, .. } => Some(*preset_id),
            _ => None,
        })
        .expect("the click asked to watch");

    snapshot.watches.push(brp_room::WatchView {
        publisher: snapshot.members[0].id,
        live_id: 1,
        preset_id,
        state: brp_room::WatchState::Connecting,
        frames_decoded: 0,
        keyframe_requests: 0,
        audio: false,
    });
    let first = run_room(
        &ctx,
        timed(window(1440.0, 900.0), 62),
        &snapshot,
        &mut state,
    )
    .0;
    let tile = first.tile_rects[0].1;
    assert!(
        tile.height() < 120.0 && tile.expand(24.0).contains(card.center()),
        "the tile began at {tile:?}, far from its card around {card:?}"
    );
    let settled = settled_room_from(63, &ctx, (1440.0, 900.0), &snapshot, &mut state).0;
    assert!(settled.tile_rects[0].1.height() > 500.0);
}

#[test]
fn the_accent_marks_the_next_thing_to_do_and_leaves_once_you_are_sharing() {
    let run = |snapshot: &brp_room::RoomSnapshot| {
        let ctx = egui::Context::default();
        brp_app::ui::theme::install(&ctx);
        let mut state = brp_app::ui::state::UiState::new();
        settled_room(&ctx, (1440.0, 900.0), snapshot, &mut state).1
    };
    assert_eq!(
        accent_surfaces(&run(&room_with_an_unwatched_stream())),
        1,
        "not sharing: the dock's Share is the way in"
    );
    let mut sharing = busy_room();
    sharing.members.clear();
    sharing.watches.clear();
    assert_eq!(accent_surfaces(&run(&sharing)), 0, "already sharing");
}

#[test]
fn the_tone_of_a_control_eases_rather_than_cutting() {
    use brp_app::ui::theme;
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let fill_at = |frame: u32, tone: theme::Tone| {
        let mut output = ctx.run_ui(timed(window(400.0, 200.0), frame), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                theme::wide_action(
                    ui,
                    theme::icons::CREATE,
                    "Act",
                    tone,
                    true,
                    egui::vec2(200.0, 36.0),
                );
            });
        });
        output.textures_delta.clear();
        // A control's backing is the one group of shapes in this window.
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Vec(group) => group.iter().find_map(|part| match part {
                egui::Shape::Rect(rect) => Some(rect.fill),
                _ => None,
            }),
            _ => None,
        })
    };
    for frame in 0..30 {
        fill_at(frame, theme::Tone::Primary);
    }
    assert_eq!(fill_at(30, theme::Tone::Primary), Some(theme::ACCENT));
    let next = fill_at(31, theme::Tone::Neutral).expect("a surface");
    assert_ne!(next, theme::RAISED, "cut straight to the new tone");
    assert_ne!(next, theme::ACCENT, "did not start moving");
    for frame in 32..90 {
        fill_at(frame, theme::Tone::Neutral);
    }
    assert_eq!(fill_at(90, theme::Tone::Neutral), Some(theme::RAISED));
}

#[test]
fn the_audio_switch_and_its_chooser_are_one_control_with_two_targets() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let snapshot = busy_room();
    let mut state = brp_app::ui::state::UiState::new();
    let (_, shapes) = settled_room(&ctx, (1440.0, 900.0), &snapshot, &mut state);
    assert!(text_rects(&shapes, "Applications…").is_empty());
    let label = text_rect(&shapes, "Share audio");
    let chooser = text_rect(&shapes, brp_app::ui::theme::icons::APPLICATIONS);
    assert!(
        (chooser.center().y - label.center().y).abs() < 2.0,
        "{chooser:?} beside {label:?}"
    );
    assert!(
        chooser.left() - label.right() < 24.0,
        "they must read as one"
    );
    let commands = click_text(
        &ctx,
        60,
        (1440.0, 900.0),
        label.center(),
        &snapshot,
        &mut state,
    );
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, brp_app::commands::RoomCommand::SetAudio(false)))
    );
    let commands = click_text(
        &ctx,
        70,
        (1440.0, 900.0),
        chooser.center(),
        &snapshot,
        &mut state,
    );
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, brp_app::commands::RoomCommand::ChooseApplications))
    );
}

#[test]
fn each_quality_is_ticked_on_its_own_row_with_its_bitrate_in_plain_units() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut snapshot = busy_room();
    // Only the first template on offer, as if the sharer had unticked the rest.
    let live = &mut snapshot.own_lives[0];
    live.presets.truncate(1);
    live.info.presets.truncate(1);
    let on = live.presets[0].preset.name.clone();
    let bitrate = brp_app::ui::own_lives::bitrate(u64::from(live.presets[0].preset.bitrate_kbps));
    let mut state = brp_app::ui::state::UiState::new();
    state.quality_open.insert(live.info.id);
    let (_, shapes) = settled_room(&ctx, (1440.0, 1000.0), &snapshot, &mut state);
    let (tick, cost) = (text_rect(&shapes, &on), text_rect(&shapes, &bitrate));
    assert!(
        (tick.center().y - cost.center().y).abs() < 2.0,
        "{tick:?} and {cost:?} should share a row"
    );
    assert!(bitrate.ends_with("Mbps"), "was {bitrate}");
    let off = text_rect(&shapes, "480p");
    assert!(off.top() > tick.bottom(), "unticked qualities stay listed");
    assert!(text_rects(&shapes, "Templates").is_empty());
}

#[test]
fn a_sheet_is_only_as_tall_as_what_it_holds_so_the_video_keeps_the_rest() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let snapshot = busy_room();
    let mut state = brp_app::ui::state::UiState::new();
    state.sheet = brp_app::ui::state::CompactSheet::Share;
    state.sheet_open = true;
    let (output, shapes) = settled_room(&ctx, (720.0, 900.0), &snapshot, &mut state);
    let figure = text_rect(&shapes, "0 kbps");
    let dock = text_rect(&shapes, "Room");
    assert!(
        dock.top() - figure.bottom() < 56.0,
        "{} of nothing under the sheet's last line",
        dock.top() - figure.bottom()
    );
    let heading = text_rect(&shapes, "Audio");
    assert!(
        heading.top() > 900.0 * 0.45,
        "the sheet still reaches {}",
        heading.top()
    );
    assert!(
        output.tile_rects[0].1.width() > 690.0,
        "the picture regains its width"
    );
}

#[test]
fn a_sheet_with_more_than_fits_stops_at_its_limit_and_scrolls() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let snapshot = busy_room();
    let mut state = brp_app::ui::state::UiState::new();
    state.sheet = brp_app::ui::state::CompactSheet::Share;
    state.sheet_open = true;
    let (output, _) = settled_room(&ctx, (720.0, 320.0), &snapshot, &mut state);
    let tile = output.tile_rects[0].1;
    assert!(tile.height() > 40.0, "the video is still there: {tile:?}");
}

#[test]
fn a_path_that_resolves_is_watched_closing_rather_than_cut_to() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut snapshot = busy_room();
    snapshot.members[0].path = brp_net::PathKind::Unknown;
    let mut state = brp_app::ui::state::UiState::new();
    let lines = |shapes: &[egui::epaint::ClippedShape]| {
        fn count(shape: &egui::Shape) -> usize {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().map(count).sum(),
                egui::Shape::LineSegment { .. } => 1,
                _ => 0,
            }
        }
        shapes
            .iter()
            .map(|shape| count(&shape.shape))
            .sum::<usize>()
    };
    let (_, dashed) = settled_room(&ctx, (1440.0, 900.0), &snapshot, &mut state);
    let unknown = lines(&dashed);
    snapshot.members[0].path = brp_net::PathKind::Direct;
    let (_, closing) = run_room(
        &ctx,
        timed(window(1440.0, 900.0), 60),
        &snapshot,
        &mut state,
    );
    assert!(
        lines(&closing) >= unknown,
        "the dashes must not vanish at once"
    );
    let (_, solid) = settled_room_from(61, &ctx, (1440.0, 900.0), &snapshot, &mut state);
    assert!(
        lines(&solid) < unknown,
        "the dashes must close into one line"
    );
}

#[test]
fn a_tile_that_finishes_leaving_in_a_discarded_pass_still_has_its_video_released() {
    let ctx = egui::Context::default();
    brp_app::ui::theme::install(&ctx);
    let mut snapshot = busy_room();
    let closing = (snapshot.watches[0].publisher, snapshot.watches[0].live_id);
    let mut state = brp_app::ui::state::UiState::new();
    settled_room(&ctx, (1440.0, 900.0), &snapshot, &mut state);
    snapshot.watches.clear();
    // Every frame runs twice and only the second pass's output is kept, as the window loop does.
    ctx.options_mut(|options| options.max_passes = std::num::NonZeroUsize::new(2).expect("two"));
    for frame in 60..180 {
        let mut kept = None;
        let mut output = ctx.run_ui(timed(window(1440.0, 900.0), frame), |ui| {
            ui.ctx().request_discard("as a resizing layout would");
            kept = Some(brp_app::ui::draw(
                ui,
                &snapshot,
                "ticket",
                &mut state,
                &std::collections::HashSet::new(),
                &UpdateState::new(false),
            ));
        });
        output.textures_delta.clear();
    }
    assert_eq!(state.gone_tiles, vec![closing]);
}
