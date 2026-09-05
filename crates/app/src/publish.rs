use crate::{cli::PublishArgs, error::AppError, identity};
use brp_capture::{CaptureBackend, PortalCapture, SourceRequest};
use brp_codec::ffmpeg::SwsConverter;
use brp_codec::{EncoderConfig, open_encoder_auto};
use brp_net::{MediaServer, RelaySetting, bind_endpoint};
use brp_pipeline::Publisher;
use brp_proto::constants::{MEDIA_ALPN, RELAY_ONLINE_TIMEOUT, STATS_LOG_INTERVAL};
use brp_proto::{Codec, PixelFormat, Preset, RoomTicket, default_bitrate_kbps};
use iroh::protocol::Router;
use std::sync::Arc;
use std::sync::atomic::Ordering;

const LIVE_ID: u32 = 1;
const PRESET_ID: u32 = 1;

pub async fn run(args: PublishArgs) -> Result<(), AppError> {
    let relay = if args.no_relay {
        RelaySetting::Disabled
    } else {
        RelaySetting::Default
    };
    let endpoint = bind_endpoint(identity::load_or_create()?, relay).await?;
    let slot: Arc<brp_pipeline::LatestSlot<Arc<brp_capture::CaptureFrame>>> =
        brp_pipeline::LatestSlot::new();
    let sink_slot = slot.clone();
    let session = PortalCapture
        .start(
            SourceRequest {
                kind: args.source.into(),
                target_fps: args.fps,
            },
            Box::new(move |frame| sink_slot.put(Arc::new(frame))),
        )
        .await?;
    let info = session.info();
    let fps = info.fps.min(args.fps).max(1);
    let (width, height) = (info.width & !1, info.height & !1);
    let bitrate_kbps = args
        .bitrate_kbps
        .unwrap_or_else(|| default_bitrate_kbps(width, height, fps));
    let forced = args.codec.map(Into::into);
    let encoder = open_encoder_auto(
        EncoderConfig {
            width,
            height,
            fps,
            bitrate_kbps,
            codec: forced.unwrap_or(Codec::Hevc),
        },
        forced,
    )?;
    let preset = Preset {
        id: PRESET_ID,
        name: "Source".into(),
        width,
        height,
        fps,
        bitrate_kbps,
        codec: encoder.params().codec,
    };
    preset.validate(info.width, info.height, info.fps.max(fps))?;
    let converter = SwsConverter::new(info.width, info.height, PixelFormat::Bgrx, width, height)?;
    let publisher = Publisher::start(LIVE_ID, PRESET_ID, slot, Box::new(converter), encoder);
    let router = Router::builder(endpoint.clone())
        .accept(MEDIA_ALPN, MediaServer::new(Arc::new(publisher.clone())))
        .spawn();
    if relay == RelaySetting::Default
        && tokio::time::timeout(RELAY_ONLINE_TIMEOUT, endpoint.online())
            .await
            .is_err()
    {
        tracing::warn!(
            "relay registration timed out; the ticket may only work on the local network"
        );
    }
    let ticket = RoomTicket::new(RoomTicket::random_topic(), vec![endpoint.addr()]);
    println!(
        "Encoder: {} ({:?} {}x{} @ {} fps, {} kbps)",
        publisher.encoder_name(),
        preset.codec,
        width,
        height,
        fps,
        bitrate_kbps
    );
    println!(
        "Ticket:\n{ticket}\n\nShare the ticket with a viewer: brp watch <ticket>. Press Ctrl-C to stop."
    );
    let mut ticker = tokio::time::interval(STATS_LOG_INTERVAL);
    let mut last_bytes = 0;
    loop {
        tokio::select! { _ = tokio::signal::ctrl_c() => break, _ = ticker.tick() => { let bytes = publisher.stats().bytes_encoded.load(Ordering::Relaxed); let kbps = (bytes.saturating_sub(last_bytes) * 8) / 1000 / STATS_LOG_INTERVAL.as_secs().max(1); last_bytes = bytes; tracing::info!(viewers = publisher.subscriber_count(), frames = publisher.stats().frames_encoded.load(Ordering::Relaxed), dropped_at_input = publisher.frames_dropped_at_input(), kbps, "publishing"); } }
    }
    publisher.stop();
    session.stop();
    if let Err(e) = router.shutdown().await {
        tracing::warn!(error = %e, "router shutdown");
    }
    endpoint.close().await;
    Ok(())
}
