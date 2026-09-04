//! Gated checks for the codecs installed on the host.

use brp_codec::select::{open_encoder, open_encoder_auto};
use brp_codec::{CodecError, EncoderConfig, RawFrame, VideoEncoder};
use brp_proto::Codec;

fn cfg(codec: Codec) -> EncoderConfig {
    EncoderConfig {
        width: 320,
        height: 240,
        fps: 30,
        bitrate_kbps: 2_000,
        codec,
    }
}
fn gated() -> bool {
    std::env::var_os("BRP_CODEC_TESTS").is_some()
}

#[test]
fn opening_an_encoder_never_panics() {
    for codec in [Codec::Hevc, Codec::H264, Codec::Av1] {
        match open_encoder(&cfg(codec)) {
            Ok(encoder) => eprintln!("{codec:?}: {}", encoder.name()),
            Err(CodecError::NoEncoder(found)) => assert_eq!(found, codec),
            Err(error) => panic!("unexpected error {error}"),
        }
    }
}

#[test]
fn auto_selection_falls_back_to_some_encoder() {
    if gated() {
        let encoder =
            open_encoder_auto(cfg(Codec::Hevc), None).expect("an encoder should be available");
        eprintln!(
            "auto picked {} ({:?})",
            encoder.name(),
            encoder.params().codec
        );
    }
}

#[test]
fn every_available_encoder_produces_keyframes() {
    if !gated() {
        return;
    }
    for codec in [Codec::Hevc, Codec::H264, Codec::Av1] {
        let Ok(mut encoder) = open_encoder(&cfg(codec)) else {
            continue;
        };
        let mut packets = Vec::new();
        for index in 0..10_u64 {
            packets.extend(
                encoder
                    .encode(&RawFrame::black(320, 240, index * 33_333), false)
                    .unwrap(),
            );
        }
        assert!(!packets.is_empty());
        assert!(packets[0].keyframe);
        assert!(
            packets
                .windows(2)
                .all(|pair| pair[1].seq == pair[0].seq + 1)
        );
        assert!(
            encoder
                .encode(&RawFrame::black(320, 240, 999_999), true)
                .unwrap()
                .iter()
                .any(|packet| packet.keyframe)
        );
    }
}

#[test]
fn vaapi_encoder_encodes_when_available() {
    if !gated() {
        return;
    }
    let mut encoder = match brp_codec::ffmpeg::VaapiEncoder::open("hevc_vaapi", &cfg(Codec::Hevc))
        .or_else(|_| brp_codec::ffmpeg::VaapiEncoder::open("h264_vaapi", &cfg(Codec::H264)))
    {
        Ok(encoder) => encoder,
        Err(error) => {
            eprintln!("skipping: no VAAPI encoder ({error})");
            return;
        }
    };
    let mut packets = Vec::new();
    for index in 0..10_u64 {
        packets.extend(
            encoder
                .encode(&RawFrame::black(320, 240, index), false)
                .unwrap(),
        );
    }
    assert!(!packets.is_empty() && packets[0].keyframe);
}

#[test]
fn every_available_encoder_round_trips_through_decoder() {
    if !gated() {
        return;
    }
    for codec in [Codec::Hevc, Codec::H264, Codec::Av1] {
        let Ok(mut encoder) = open_encoder(&cfg(codec)) else {
            continue;
        };
        let mut decoder =
            brp_codec::open_decoder(&encoder.params()).expect("decoder for available encoder");
        let mut decoded = 0;
        for index in 0..12_u64 {
            for packet in encoder
                .encode(&RawFrame::black(320, 240, index * 10), false)
                .unwrap()
            {
                for raw in decoder.decode(&packet).unwrap() {
                    assert_eq!((raw.width, raw.height), (320, 240));
                    raw.validate().unwrap();
                    assert!(raw.y[0] < 40);
                    decoded += 1;
                }
            }
        }
        assert!(
            decoded >= 8,
            "{} produced only {decoded} frames",
            encoder.name()
        );
    }
}
