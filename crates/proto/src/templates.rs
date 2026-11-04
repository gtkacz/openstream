use crate::bitrate::default_bitrate_kbps;
use crate::constants::{
    CHROMA_444_PRESET_ID, LOSSLESS_PRESET_ID, MAX_BITRATE_KBPS, SOURCE_PRESET_ID, TEMPLATE_HEIGHTS,
};
use crate::messages::{Codec, Preset, VideoQuality};

/// Source at even dimensions, then one derived preset per template height strictly below the
/// source, with stable ids so a viewer's choice survives the publisher toggling other templates.
pub fn template_presets(
    source_width: u32,
    source_height: u32,
    fps: u32,
    codec: Codec,
) -> Vec<Preset> {
    let (width, height) = (source_width & !1, source_height & !1);
    let mut presets = vec![Preset {
        id: SOURCE_PRESET_ID,
        name: "Source".into(),
        width,
        height,
        fps,
        bitrate_kbps: default_bitrate_kbps(width, height, fps),
        codec,
    }];
    for (index, &template_height) in TEMPLATE_HEIGHTS.iter().enumerate() {
        if template_height >= height {
            continue;
        }
        let template_width =
            (u64::from(width) * u64::from(template_height) / u64::from(height)) as u32 & !1;
        presets.push(Preset {
            id: SOURCE_PRESET_ID + 1 + index as u32,
            name: format!("{template_height}p"),
            width: template_width,
            height: template_height,
            fps,
            bitrate_kbps: default_bitrate_kbps(template_width, template_height, fps),
            codec,
        });
    }
    presets
}

/// The full-chroma presets, at source size. Offered for a publisher to tick rather than among the
/// defaults, because only some encoders can produce them and a viewer who picked one could fail.
pub fn quality_presets(
    source_width: u32,
    source_height: u32,
    fps: u32,
    codec: Codec,
) -> Vec<Preset> {
    let (width, height) = (source_width & !1, source_height & !1);
    let preset = |id, name: &str, bitrate_kbps, quality: VideoQuality| Preset {
        id,
        name: name.into(),
        width,
        height,
        fps,
        bitrate_kbps,
        codec: quality.codec_for(codec),
    };
    // 4:4:4 carries twice the samples of 4:2:0 but they compress better than luma.
    let chroma_kbps = (default_bitrate_kbps(width, height, fps) * 3 / 2).min(MAX_BITRATE_KBPS);
    vec![
        preset(
            CHROMA_444_PRESET_ID,
            "Source 4:4:4",
            chroma_kbps,
            VideoQuality::Chroma444,
        ),
        // The encoder ignores this; it only has to pass `Preset::validate`.
        preset(
            LOSSLESS_PRESET_ID,
            "Source lossless",
            MAX_BITRATE_KBPS,
            VideoQuality::Lossless,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Codec;
    use crate::constants::SOURCE_PRESET_ID;

    #[test]
    fn source_plus_every_smaller_template_with_even_aspect_preserving_widths() {
        let presets = template_presets(2560, 1440, 60, Codec::Hevc);
        let dims: Vec<(u32, &str, u32, u32)> = presets
            .iter()
            .map(|p| (p.id, p.name.as_str(), p.width, p.height))
            .collect();
        assert_eq!(
            dims,
            vec![
                (SOURCE_PRESET_ID, "Source", 2560, 1440),
                (2, "1080p", 1920, 1080),
                (3, "720p", 1280, 720),
                (4, "480p", 852, 480)
            ]
        );
        assert!(
            presets
                .iter()
                .all(|p| p.codec == Codec::Hevc && p.fps == 60)
        );
        assert_eq!(presets[1].bitrate_kbps, 20_000);
    }

    #[test]
    fn quality_presets_stay_at_source_size_and_never_ask_for_av1() {
        let presets = quality_presets(2561, 1441, 60, Codec::Av1);
        assert_eq!(
            presets
                .iter()
                .map(|p| (p.id, p.quality(), p.width, p.height, p.codec))
                .collect::<Vec<_>>(),
            vec![
                (
                    CHROMA_444_PRESET_ID,
                    VideoQuality::Chroma444,
                    2560,
                    1440,
                    Codec::Hevc
                ),
                (
                    LOSSLESS_PRESET_ID,
                    VideoQuality::Lossless,
                    2560,
                    1440,
                    Codec::Hevc
                ),
            ]
        );
        assert!(presets.iter().all(|p| p.validate(2561, 1441, 60).is_ok()));
        assert_eq!(
            quality_presets(64, 32, 60, Codec::H264)[0].codec,
            Codec::H264
        );
    }

    #[test]
    fn every_template_together_fits_the_presence_limit() {
        let all = template_presets(3840, 2160, 60, Codec::Hevc).len()
            + quality_presets(3840, 2160, 60, Codec::Hevc).len();
        assert!(all <= crate::constants::MAX_PRESETS_PER_LIVE, "{all}");
    }

    #[test]
    fn default_presets_are_all_standard_quality() {
        assert!(
            template_presets(3840, 2160, 60, Codec::Hevc)
                .iter()
                .all(|p| p.quality() == VideoQuality::Standard)
        );
    }

    #[test]
    fn odd_source_dimensions_round_down_and_equal_heights_are_not_offered() {
        let presets = template_presets(1281, 721, 30, Codec::H264);
        assert_eq!((presets[0].width, presets[0].height), (1280, 720));
        assert_eq!(presets.len(), 2, "only 480p is strictly smaller than 721");
        assert_eq!(
            (presets[1].id, presets[1].width, presets[1].height),
            (4, 852, 480)
        );
    }
}
