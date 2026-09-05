use std::str::FromStr;
use std::sync::Arc;

use brp_capture::PortalCapture;
use brp_net::RelaySetting;
use brp_proto::constants::{RELAY_ONLINE_TIMEOUT, SOURCE_PRESET_ID, STATS_LOG_INTERVAL};
use brp_proto::{RoomTicket, SourceKind};
use brp_room::codecs::FfmpegCodecs;
use brp_room::{Room, RoomConfig, RoomTimings};

use crate::cli::PublishArgs;
use crate::error::AppError;
use crate::identity;

pub async fn run(args: PublishArgs) -> Result<(), AppError> {
    let relay = if args.no_relay {
        RelaySetting::Disabled
    } else {
        RelaySetting::Default
    };
    let secret = identity::load_or_create()?;
    let nickname = args
        .nickname
        .clone()
        .unwrap_or_else(|| secret.public().fmt_short().to_string());
    let config = RoomConfig {
        secret,
        relay,
        nickname,
        target_fps: args.fps,
        capture: Arc::new(PortalCapture),
        encoders: Arc::new(FfmpegCodecs::default()),
        decoders: Arc::new(FfmpegCodecs::default()),
        on_change: Arc::new(|| {}),
        on_frame: Arc::new(|| {}),
        timings: RoomTimings::default(),
    };
    let room = match &args.ticket {
        Some(ticket) => Room::join(config, RoomTicket::from_str(ticket)?).await?,
        None => Room::create(config).await?,
    };

    let kind: SourceKind = args.source.into();
    let title = match kind {
        SourceKind::Monitor => "Monitor 1",
        SourceKind::Window => "Window 1",
    };
    let live = room.start_live(kind, title.into()).await?;
    if args.bitrate_kbps.is_some() || args.codec.is_some() {
        let mut presets = room
            .snapshot()
            .own_lives
            .iter()
            .find(|l| l.info.id == live)
            .map(|l| l.info.presets.clone())
            .unwrap_or_default();
        for preset in &mut presets {
            if let (Some(bitrate), true) = (args.bitrate_kbps, preset.id == SOURCE_PRESET_ID) {
                preset.bitrate_kbps = bitrate;
            }
            if let Some(codec) = args.codec {
                preset.codec = codec.into();
            }
        }
        room.set_presets(live, presets)?;
    }

    if relay == RelaySetting::Default && !room.online(RELAY_ONLINE_TIMEOUT).await {
        tracing::warn!(
            "relay registration timed out; the ticket may only work on the local network"
        );
    }
    let snapshot = room.snapshot();
    let own = &snapshot.own_lives[0];
    println!(
        "Nickname: {}  Live: {} ({}x{} @ {} fps, {} presets)",
        snapshot.nickname,
        own.info.title,
        own.info.source_width,
        own.info.source_height,
        own.info.source_fps,
        own.presets.len()
    );
    println!(
        "Ticket:\n{}\n\nShare it: brp watch <ticket>. Press Ctrl-C to stop.",
        room.ticket()
    );

    let mut ticker = tokio::time::interval(STATS_LOG_INTERVAL);
    let mut last_bytes = 0u64;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = ticker.tick() => {
                let snapshot = room.snapshot();
                let bytes: u64 = snapshot.own_lives.iter().flat_map(|l| l.presets.iter()).filter_map(|p| p.encoder.as_ref()).map(|e| e.bytes_encoded).sum();
                let kbps = bytes.saturating_sub(last_bytes) * 8 / 1000 / STATS_LOG_INTERVAL.as_secs().max(1);
                last_bytes = bytes;
                let running: Vec<String> = snapshot.own_lives.iter().flat_map(|l| l.presets.iter()).filter_map(|p| p.encoder.as_ref().map(|e| format!("{}:{}x{}", e.name, p.preset.width, p.preset.height))).collect();
                tracing::info!(members = snapshot.members.len(), encoders = ?running, kbps, "publishing");
            }
        }
    }
    room.leave().await;
    Ok(())
}
