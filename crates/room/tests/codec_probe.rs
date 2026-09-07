//! Gated check that the production codec probe agrees with a real-size encoder open.

use brp_codec::{EncoderConfig, open_encoder_auto};
use brp_proto::Codec;
use brp_room::codecs::{EncoderFactory, FfmpegCodecs};

fn gated() -> bool {
    std::env::var_os("BRP_CODEC_TESTS").is_some()
}

#[test]
fn preferred_codec_matches_what_a_720p_open_selects() {
    if !gated() {
        return;
    }
    let real_size = EncoderConfig {
        width: 1280,
        height: 720,
        fps: 30,
        bitrate_kbps: 5_000,
        codec: Codec::Hevc,
    };
    let expected = open_encoder_auto(real_size, None)
        .expect("an encoder should be available")
        .params()
        .codec;
    assert_eq!(FfmpegCodecs::default().preferred_codec(), expected);
}
