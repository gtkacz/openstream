//! Gated checks for the codecs installed on the host.

use brp_codec::ffmpeg::{FfmpegDecoder, HwDecode};
use brp_codec::select::{open_encoder, open_encoder_auto};
use brp_codec::{CodecError, EncoderConfig, RawFrame, VideoDecoder, VideoEncoder};
use brp_proto::{ChromaLayout, Codec, EncodedFrame, VideoQuality};

fn cfg(codec: Codec) -> EncoderConfig {
    EncoderConfig {
        width: 320,
        height: 240,
        fps: 30,
        bitrate_kbps: 2_000,
        codec,
        quality: brp_proto::VideoQuality::Standard,
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

/// On a Mac, HEVC and H.264 must encode and decode on VideoToolbox, not fall through to software.
#[cfg(target_os = "macos")]
#[test]
fn macos_encodes_and_decodes_hevc_and_h264_on_videotoolbox() {
    use brp_codec::ffmpeg::{FfmpegDecoder, HwDecode};
    if !gated() {
        return;
    }
    for (codec, expected) in [
        (Codec::Hevc, "hevc_videotoolbox"),
        (Codec::H264, "h264_videotoolbox"),
    ] {
        let mut encoder = open_encoder(&cfg(codec)).expect("VideoToolbox encoder");
        assert_eq!(encoder.name(), expected);
        let mut decoder = FfmpegDecoder::open(&encoder.params(), HwDecode::Auto).expect("decoder");
        assert_eq!(
            decoder.hardware_device(),
            Some("videotoolbox"),
            "{codec:?} decodes in software"
        );
        let mut decoded = 0;
        for index in 0..12_u64 {
            for packet in encoder
                .encode(&RawFrame::black(320, 240, index * 33_333), false)
                .unwrap()
            {
                use brp_codec::VideoDecoder;
                decoded += decoder.decode(&packet).unwrap().len();
            }
        }
        eprintln!("{expected}: {decoded} frames decoded on videotoolbox");
        assert!(decoded >= 8, "{expected} decoded only {decoded} frames");
    }
}

/// A frame whose chroma changes every column, so a round trip that averaged it would show.
fn colourful(index: u64) -> RawFrame {
    let mut frame = RawFrame::black_with(ChromaLayout::Full, 320, 240, index * 33_333);
    for row in 0..240 {
        for column in 0..320 {
            let shift = column + row + index as usize;
            frame.y[row * 320 + column] = 16 + (shift % 200) as u8;
            frame.uv[row * 640 + column * 2] = if column % 2 == 0 { 60 } else { 200 };
            frame.uv[row * 640 + column * 2 + 1] = if column % 2 == 0 { 200 } else { 60 };
        }
    }
    frame
}

fn round_trip(codec: Codec, quality: VideoQuality, hw: HwDecode) -> Option<(RawFrame, RawFrame)> {
    let config = EncoderConfig {
        quality,
        ..cfg(codec)
    };
    let mut encoder = match open_encoder(&config) {
        Ok(encoder) => encoder,
        Err(CodecError::NoFullChromaEncoder(found)) => {
            assert_eq!(found, codec);
            return None;
        }
        Err(error) => panic!("unexpected error {error}"),
    };
    let mut decoder = FfmpegDecoder::open(&encoder.params(), hw).unwrap();
    let mut sent = Vec::new();
    let mut received = Vec::new();
    for index in 0..10 {
        let frame = colourful(index);
        for packet in encoder.encode(&frame, index == 0).unwrap() {
            received.extend(decoder.decode(&packet).unwrap());
        }
        sent.push(frame);
    }
    let last = received.pop()?;
    let matching = sent
        .into_iter()
        .find(|frame| frame.capture_ts_us == last.capture_ts_us)
        .expect("the decoder only reports timestamps it was given");
    Some((matching, last))
}

#[test]
fn full_chroma_survives_the_encoder_and_decoder() {
    if !gated() {
        return;
    }
    for codec in [Codec::Hevc, Codec::H264] {
        let Some((sent, got)) = round_trip(codec, VideoQuality::Chroma444, HwDecode::Software)
        else {
            eprintln!("{codec:?}: no 4:4:4 encoder here");
            continue;
        };
        assert_eq!(got.chroma, ChromaLayout::Full, "{codec:?}");
        got.validate().unwrap();
        let (u_even, u_odd) = (got.uv[0], got.uv[2]);
        assert!(
            u_even < 110 && u_odd > 150,
            "{codec:?}: neighbouring columns must keep their own chroma, got {u_even} and {u_odd}"
        );
        assert_eq!(sent.uv.len(), got.uv.len());
    }
}

#[test]
fn lossless_returns_the_exact_samples_on_every_decode_path() {
    if !gated() {
        return;
    }
    for codec in [Codec::Hevc, Codec::H264] {
        for hw in [HwDecode::Software, HwDecode::Auto] {
            let Some((sent, got)) = round_trip(codec, VideoQuality::Lossless, hw) else {
                eprintln!("{codec:?}: no lossless encoder here");
                continue;
            };
            assert_eq!(got.chroma, ChromaLayout::Full, "{codec:?} {hw:?}");
            assert!(got.y == sent.y, "{codec:?} {hw:?}: luma differs");
            assert!(got.uv == sent.uv, "{codec:?} {hw:?}: chroma differs");
        }
    }
}

#[test]
fn an_encoder_refuses_a_frame_of_the_wrong_chroma() {
    if !gated() {
        return;
    }
    let Ok(mut encoder) = open_encoder(&cfg(Codec::H264)) else {
        return;
    };
    assert!(matches!(
        encoder.encode(&colourful(0), false),
        Err(CodecError::InvalidFrame(_))
    ));
}

#[test]
fn av1_at_full_chroma_says_so_instead_of_falling_back() {
    let config = EncoderConfig {
        quality: VideoQuality::Chroma444,
        ..cfg(Codec::Av1)
    };
    assert!(matches!(
        open_encoder(&config),
        Err(CodecError::NoFullChromaEncoder(Codec::Av1))
    ));
}

/// An IDR slice header followed by noise. FFmpeg takes it without error and produces nothing, which
/// is how many hardware refusals look.
fn corrupt_keyframe() -> EncodedFrame {
    let mut data = vec![0, 0, 0, 1, 0x26, 0x01];
    data.extend(std::iter::repeat_n(0xFF, 512));
    EncodedFrame {
        seq: 0,
        capture_ts_us: 0,
        keyframe: true,
        data,
    }
}

#[test]
fn a_hardware_decoder_that_never_produces_a_frame_carries_on_in_software() {
    if !gated() {
        return;
    }
    let Ok(mut encoder) = open_encoder(&cfg(Codec::Hevc)) else {
        return;
    };
    let mut decoder = FfmpegDecoder::open(&encoder.params(), HwDecode::Auto).unwrap();
    if !decoder.is_hardware() {
        eprintln!("no hardware decoder here");
        return;
    }
    let pool = decoder.pool().unwrap();
    let mut last = Ok(Vec::new());
    for _ in 0..30 {
        assert!(decoder.is_hardware(), "gave up on the hardware too early");
        last = decoder.decode(&corrupt_keyframe());
    }
    assert!(!decoder.is_hardware(), "the fallback happened");
    assert!(
        last.is_err(),
        "the viewer must be told to ask for a keyframe"
    );
    assert!(
        std::sync::Arc::ptr_eq(&pool, &decoder.pool().unwrap()),
        "whoever displays frames still returns them to the same pool"
    );

    let keyframe = RawFrame::black(320, 240, 1);
    let decoded: Vec<RawFrame> = encoder
        .encode(&keyframe, true)
        .unwrap()
        .iter()
        .flat_map(|packet| decoder.decode(packet).unwrap())
        .collect();
    assert_eq!(decoded.len(), 1, "software picks up at the next keyframe");
}

#[test]
fn packets_before_the_first_keyframe_do_not_count_against_the_hardware_decoder() {
    if !gated() {
        return;
    }
    let Ok(encoder) = open_encoder(&cfg(Codec::Hevc)) else {
        return;
    };
    let mut decoder = FfmpegDecoder::open(&encoder.params(), HwDecode::Auto).unwrap();
    if !decoder.is_hardware() {
        return;
    }
    let late_keyframe = EncodedFrame {
        keyframe: false,
        ..corrupt_keyframe()
    };
    for _ in 0..60 {
        let _ = decoder.decode(&late_keyframe);
    }
    assert!(decoder.is_hardware());
}

#[test]
fn a_bad_packet_after_decoding_has_started_does_not_cost_the_hardware_decoder() {
    if !gated() {
        return;
    }
    let Ok(mut encoder) = open_encoder(&cfg(Codec::Hevc)) else {
        return;
    };
    let mut decoder = FfmpegDecoder::open(&encoder.params(), HwDecode::Auto).unwrap();
    if !decoder.is_hardware() {
        return;
    }
    for packet in encoder.encode(&RawFrame::black(320, 240, 1), true).unwrap() {
        decoder.decode(&packet).unwrap();
    }
    let lost_reference = EncodedFrame {
        keyframe: false,
        ..corrupt_keyframe()
    };
    for _ in 0..60 {
        let _ = decoder.decode(&lost_reference);
        let _ = decoder.decode(&corrupt_keyframe());
    }
    assert!(decoder.is_hardware(), "this decoder has shown it can do it");
}
