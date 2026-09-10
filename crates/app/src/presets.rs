//! Pure edits to one live's preset list. The bottom panel calls these and hands the result to
//! `Room::set_presets`, which validates and restarts the affected encoders.

use brp_proto::constants::{MAX_BITRATE_KBPS, MIN_BITRATE_KBPS};
use brp_proto::{Codec, LiveInfo, Preset, template_presets};

/// The presets the templates offer for this live, Source included, at the frame rate and codec
/// of its current presets. Empty when the live offers nothing, which the registry refuses.
pub fn templates_for(info: &LiveInfo) -> Vec<Preset> {
    let Some(current) = info.presets.first() else {
        return Vec::new();
    };
    template_presets(
        info.source_width,
        info.source_height,
        current.fps,
        current.codec,
    )
}

/// Adds the template when absent, removes it when present. The last preset stays, so a live
/// always offers something to watch.
pub fn toggle_template(info: &LiveInfo, template_id: u32) -> Vec<Preset> {
    let mut presets = info.presets.clone();
    if let Some(position) = presets.iter().position(|p| p.id == template_id) {
        if presets.len() > 1 {
            presets.remove(position);
        }
    } else if let Some(template) = templates_for(info)
        .into_iter()
        .find(|p| p.id == template_id)
    {
        presets.push(template);
        presets.sort_by_key(|p| p.id);
    }
    presets
}

/// Sets the bitrate on the preset matching `preset_id`, clamped to the allowed range; other
/// presets are left untouched.
pub fn with_bitrate(info: &LiveInfo, preset_id: u32, kbps: u32) -> Vec<Preset> {
    info.presets
        .iter()
        .cloned()
        .map(|mut preset| {
            if preset.id == preset_id {
                preset.bitrate_kbps = kbps.clamp(MIN_BITRATE_KBPS, MAX_BITRATE_KBPS);
            }
            preset
        })
        .collect()
}

/// Sets every preset's frame rate, clamped to one through the source rate.
pub fn with_fps(info: &LiveInfo, fps: u32) -> Vec<Preset> {
    let fps = fps.clamp(1, info.source_fps.max(1));
    info.presets
        .iter()
        .cloned()
        .map(|mut preset| {
            preset.fps = fps;
            preset
        })
        .collect()
}

/// Sets the codec on every preset of the live.
pub fn with_codec(info: &LiveInfo, codec: Codec) -> Vec<Preset> {
    info.presets
        .iter()
        .cloned()
        .map(|mut preset| {
            preset.codec = codec;
            preset
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use brp_proto::SourceKind;
    use brp_proto::constants::SOURCE_PRESET_ID;

    fn live() -> LiveInfo {
        let mut presets = template_presets(2560, 1440, 60, Codec::Hevc);
        // Source plus 1080p only, so 720p and 480p are available templates.
        presets.retain(|p| p.id <= 2);
        LiveInfo {
            id: 1,
            title: "desk".into(),
            kind: SourceKind::Monitor,
            source_width: 2560,
            source_height: 1440,
            source_fps: 60,
            has_audio: false,
            presets,
        }
    }

    #[test]
    fn templates_include_source_and_follow_the_current_rate_and_codec() {
        let mut info = live();
        for preset in &mut info.presets {
            preset.fps = 30;
            preset.codec = Codec::Av1;
        }
        let templates = templates_for(&info);
        let ids: Vec<u32> = templates.iter().map(|p| p.id).collect();
        assert_eq!(ids, [1, 2, 3, 4]);
        assert!(
            templates
                .iter()
                .all(|p| p.fps == 30 && p.codec == Codec::Av1)
        );
    }

    #[test]
    fn toggling_adds_a_missing_template_in_id_order_and_removes_a_present_one() {
        let info = live();
        let added = toggle_template(&info, 4);
        assert_eq!(added.iter().map(|p| p.id).collect::<Vec<_>>(), [1, 2, 4]);
        assert_eq!((added[2].width, added[2].height), (852, 480));
        let removed = toggle_template(&info, 2);
        assert_eq!(removed.iter().map(|p| p.id).collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn source_can_be_toggled_off_while_another_preset_remains() {
        let info = live();
        let without_source = toggle_template(&info, SOURCE_PRESET_ID);
        assert_eq!(without_source.iter().map(|p| p.id).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn the_last_preset_cannot_be_toggled_off() {
        let mut info = live();
        info.presets.retain(|p| p.id == SOURCE_PRESET_ID);
        assert_eq!(toggle_template(&info, SOURCE_PRESET_ID), info.presets);
        info.presets = live().presets;
        info.presets.retain(|p| p.id == 2);
        assert_eq!(toggle_template(&info, 2), info.presets);
    }

    #[test]
    fn unknown_ids_are_ignored() {
        let info = live();
        assert_eq!(toggle_template(&info, 99), info.presets);
    }

    #[test]
    fn bitrate_applies_to_one_preset_and_clamps_to_the_allowed_range() {
        let info = live();
        let presets = with_bitrate(&info, 2, 999_999);
        assert_eq!(presets[1].bitrate_kbps, MAX_BITRATE_KBPS);
        assert_eq!(presets[0].bitrate_kbps, info.presets[0].bitrate_kbps);
        assert_eq!(with_bitrate(&info, 1, 10)[0].bitrate_kbps, MIN_BITRATE_KBPS);
    }

    #[test]
    fn frame_rate_applies_to_every_preset_within_the_source_rate() {
        let info = live();
        assert!(with_fps(&info, 30).iter().all(|p| p.fps == 30));
        assert!(with_fps(&info, 144).iter().all(|p| p.fps == 60));
        assert!(with_fps(&info, 0).iter().all(|p| p.fps == 1));
    }

    #[test]
    fn codec_applies_to_every_preset() {
        assert!(
            with_codec(&live(), Codec::H264)
                .iter()
                .all(|p| p.codec == Codec::H264)
        );
    }

    #[test]
    fn templates_follow_the_remaining_preset_when_source_is_absent() {
        let mut info = live();
        info.presets.retain(|p| p.id != SOURCE_PRESET_ID);
        for preset in &mut info.presets {
            preset.fps = 30;
            preset.codec = Codec::Av1;
        }
        let templates = templates_for(&info);
        assert_eq!(
            templates.iter().map(|p| p.id).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        assert!(
            templates
                .iter()
                .all(|p| p.fps == 30 && p.codec == Codec::Av1)
        );
        assert_eq!(
            toggle_template(&info, 4)
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            [2, 4]
        );
        let restored = toggle_template(&info, SOURCE_PRESET_ID);
        assert_eq!(restored.iter().map(|p| p.id).collect::<Vec<_>>(), [1, 2]);
        assert_eq!((restored[0].width, restored[0].height), (2560, 1440));
    }

    #[test]
    fn edits_of_an_empty_preset_list_stay_empty() {
        let mut info = live();
        info.presets = Vec::new();
        assert!(templates_for(&info).is_empty());
        assert!(with_fps(&info, 30).is_empty());
        assert!(with_codec(&info, Codec::Av1).is_empty());
        assert!(with_bitrate(&info, 1, 5_000).is_empty());
    }
}
