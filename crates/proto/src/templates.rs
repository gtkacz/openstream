use crate::bitrate::default_bitrate_kbps;
use crate::constants::{SOURCE_PRESET_ID, TEMPLATE_HEIGHTS};
use crate::messages::{Codec, Preset};

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
