use iroh_base::{PublicKey, SecretKey, Signature};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};

use crate::constants::{
    MAX_AUDIO_MIX_NAME_LEN, MAX_AUDIO_MIX_NAMES, MAX_LIVES_PER_PARTICIPANT, MAX_PRESETS_PER_LIVE,
    NICKNAME_MAX_LEN, PROTOCOL_VERSION,
};
use crate::error::ProtoError;
use crate::messages::{Preset, SourceKind, VideoQuality};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveInfo {
    pub id: u32,
    pub title: String,
    pub kind: SourceKind,
    pub source_width: u32,
    pub source_height: u32,
    pub source_fps: u32,
    pub has_audio: bool,
    pub presets: Vec<Preset>,
}

/// Which applications a publisher's audio carries, as the room is told.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioMix {
    /// Everything the machine plays. Deliberately no list: naming what happens to be playing would
    /// tell the room more than the publisher chose to share.
    All,
    /// The named applications, and how many more did not fit in a presence.
    Only { names: Vec<String>, others: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    pub version: u8,
    pub ts_unix_ms: u64,
    pub nickname: String,
    pub lives: Vec<LiveInfo>,
    /// New fields go after this line, never before. Postcard ignores trailing bytes, so a peer too
    /// old to know them still reads our presence, and this read of its shorter one runs out of
    /// input here. `None` is such a peer, which does not say.
    #[serde(deserialize_with = "absent_when_truncated")]
    pub audio_mix: Option<AudioMix>,
    /// Full-chroma presets by live id, kept out of `lives` on the wire. A peer too old to decode
    /// them would otherwise offer them and fail on every frame, and this way it never sees them.
    #[serde(deserialize_with = "absent_when_truncated")]
    pub quality_presets: Vec<(u32, Preset)>,
    /// `None` is out of voice, or a peer too old to say.
    #[serde(deserialize_with = "absent_when_truncated")]
    pub voice: Option<VoiceState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VoiceState {
    pub muted: bool,
    pub deafened: bool,
}

fn absent_when_truncated<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(
    deserializer: D,
) -> Result<T, D::Error> {
    Ok(T::deserialize(deserializer).unwrap_or_default())
}

impl Presence {
    /// Moves full-chroma presets from `lives` to `quality_presets`, for sending.
    pub fn hide_quality_presets(&mut self) {
        for live in &mut self.lives {
            let (standard, quality): (Vec<_>, Vec<_>) = std::mem::take(&mut live.presets)
                .into_iter()
                .partition(|preset| preset.quality() == VideoQuality::Standard);
            live.presets = standard;
            self.quality_presets
                .extend(quality.into_iter().map(|preset| (live.id, preset)));
        }
    }

    /// Puts them back, for reading. Run before `validate`, whose preset limit covers them too. A
    /// preset naming a live this presence does not list is dropped.
    pub fn restore_quality_presets(&mut self) {
        for (live_id, preset) in std::mem::take(&mut self.quality_presets) {
            if let Some(live) = self.lives.iter_mut().find(|live| live.id == live_id) {
                live.presets.push(preset);
            }
        }
    }

    pub fn validate(&self) -> Result<(), ProtoError> {
        if self.version != PROTOCOL_VERSION {
            return Err(ProtoError::Invalid(format!(
                "presence version {} is not {PROTOCOL_VERSION}",
                self.version
            )));
        }
        if self.nickname.chars().count() > NICKNAME_MAX_LEN {
            return Err(ProtoError::Invalid("nickname too long".into()));
        }
        if self.lives.len() > MAX_LIVES_PER_PARTICIPANT {
            return Err(ProtoError::Invalid("too many lives".into()));
        }
        if self
            .lives
            .iter()
            .any(|l| l.presets.len() > MAX_PRESETS_PER_LIVE)
        {
            return Err(ProtoError::Invalid("too many presets on a live".into()));
        }
        if let Some(AudioMix::Only { names, .. }) = &self.audio_mix
            && (names.len() > MAX_AUDIO_MIX_NAMES
                || names
                    .iter()
                    .any(|name| name.chars().count() > MAX_AUDIO_MIX_NAME_LEN))
        {
            return Err(ProtoError::Invalid("audio mix list too long".into()));
        }
        Ok(())
    }
}

/// Gossip reports the last hop, not the author, so authorship travels inside the message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signed {
    pub author: PublicKey,
    pub payload: Vec<u8>,
    pub signature: Signature,
}

impl Signed {
    pub fn sign<T: Serialize>(secret: &SecretKey, value: &T) -> Result<Self, ProtoError> {
        let payload = crate::messages::encode(value)?;
        let signature = secret.sign(&payload);
        Ok(Self {
            author: secret.public(),
            payload,
            signature,
        })
    }

    pub fn verify<T: DeserializeOwned>(&self) -> Result<T, ProtoError> {
        self.author
            .verify(&self.payload, &self.signature)
            .map_err(|_| ProtoError::BadSignature)?;
        crate::messages::decode(&self.payload)
    }
}

#[cfg(test)]
mod tests {
    use iroh_base::SecretKey;

    use super::*;
    use crate::constants::{MAX_LIVES_PER_PARTICIPANT, NICKNAME_MAX_LEN, PROTOCOL_VERSION};

    fn presence(nickname: &str, lives: usize) -> Presence {
        Presence {
            version: PROTOCOL_VERSION,
            ts_unix_ms: 1_700_000_000_000,
            nickname: nickname.into(),
            lives: (0..lives as u32)
                .map(|i| LiveInfo {
                    id: i + 1,
                    title: format!("live {i}"),
                    kind: crate::SourceKind::Monitor,
                    source_width: 1920,
                    source_height: 1080,
                    source_fps: 60,
                    has_audio: false,
                    presets: vec![],
                })
                .collect(),
            audio_mix: Some(AudioMix::All),
            quality_presets: Vec::new(),
            voice: None,
        }
    }

    fn with_quality_presets() -> Presence {
        let mut ours = presence("new", 1);
        ours.lives[0].presets = crate::template_presets(2560, 1440, 60, crate::Codec::Hevc);
        ours.lives[0]
            .presets
            .extend(crate::quality_presets(2560, 1440, 60, crate::Codec::Hevc));
        ours
    }

    #[test]
    fn an_older_peer_is_never_offered_a_preset_it_could_not_decode() {
        let mut ours = with_quality_presets();
        ours.hide_quality_presets();
        let read: OlderPresence =
            crate::messages::decode(&crate::messages::encode(&ours).unwrap()).unwrap();
        assert_eq!(
            read.lives[0]
                .presets
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
    }

    #[test]
    fn a_current_peer_gets_every_preset_back_in_order() {
        let sent = with_quality_presets();
        let mut hidden = sent.clone();
        hidden.hide_quality_presets();
        let mut read: Presence =
            crate::messages::decode(&crate::messages::encode(&hidden).unwrap()).unwrap();
        read.restore_quality_presets();
        assert_eq!(read, sent);
    }

    #[test]
    fn restored_presets_count_towards_the_preset_limit() {
        let mut crowded = with_quality_presets();
        crowded.lives[0].presets.truncate(MAX_PRESETS_PER_LIVE);
        crowded.quality_presets = vec![(1, crowded.lives[0].presets[0].clone())];
        crowded.restore_quality_presets();
        assert!(crowded.validate().is_err());
    }

    #[test]
    fn a_preset_for_a_live_not_listed_is_dropped() {
        let mut stray = with_quality_presets();
        let before = stray.lives.clone();
        stray.quality_presets = vec![(99, before[0].presets[0].clone())];
        stray.restore_quality_presets();
        assert_eq!(stray.lives, before);
        assert!(stray.quality_presets.is_empty());
    }

    /// The presence every release before `audio_mix` sends and reads.
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct OlderPresence {
        version: u8,
        ts_unix_ms: u64,
        nickname: String,
        lives: Vec<LiveInfo>,
    }

    #[test]
    fn an_older_peers_presence_reads_as_saying_nothing_about_its_mix() {
        let older = presence("old", 1);
        let bytes = crate::messages::encode(&OlderPresence {
            version: older.version,
            ts_unix_ms: older.ts_unix_ms,
            nickname: older.nickname.clone(),
            lives: older.lives.clone(),
        })
        .unwrap();
        let read: Presence = crate::messages::decode(&bytes).unwrap();
        assert_eq!(read.audio_mix, None);
        assert!(read.quality_presets.is_empty());
        assert_eq!(read.lives, older.lives);
    }

    #[test]
    fn voice_state_round_trips_in_presence() {
        let ours = Presence {
            voice: Some(VoiceState {
                muted: true,
                deafened: false,
            }),
            ..presence("talker", 0)
        };
        let read: Presence =
            crate::messages::decode(&crate::messages::encode(&ours).unwrap()).unwrap();
        assert_eq!(read, ours);
    }

    #[test]
    fn an_older_peers_presence_reads_as_not_in_voice() {
        let older = presence("old", 1);
        let bytes = crate::messages::encode(&OlderPresence {
            version: older.version,
            ts_unix_ms: older.ts_unix_ms,
            nickname: older.nickname.clone(),
            lives: older.lives.clone(),
        })
        .unwrap();
        let read: Presence = crate::messages::decode(&bytes).unwrap();
        assert_eq!(read.voice, None);
    }

    #[test]
    fn an_older_peer_can_still_read_our_presence() {
        let ours = Presence {
            audio_mix: Some(AudioMix::Only {
                names: vec!["firefox".into()],
                others: 2,
            }),
            ..presence("new", 1)
        };
        let read: OlderPresence =
            crate::messages::decode(&crate::messages::encode(&ours).unwrap()).unwrap();
        assert_eq!(read.nickname, "new");
        assert_eq!(read.lives, ours.lives);
    }

    #[test]
    fn a_named_mix_round_trips() {
        let ours = Presence {
            audio_mix: Some(AudioMix::Only {
                names: vec!["firefox".into(), "game.exe".into()],
                others: 1,
            }),
            ..presence("new", 1)
        };
        let read: Presence =
            crate::messages::decode(&crate::messages::encode(&ours).unwrap()).unwrap();
        assert_eq!(read, ours);
    }

    #[test]
    fn validation_bounds_what_a_peer_can_name() {
        let with = |names: Vec<String>| Presence {
            audio_mix: Some(AudioMix::Only { names, others: 0 }),
            ..presence("gt", 0)
        };
        assert!(
            with(vec!["a".repeat(MAX_AUDIO_MIX_NAME_LEN)])
                .validate()
                .is_ok()
        );
        assert!(
            with(vec!["a".repeat(MAX_AUDIO_MIX_NAME_LEN + 1)])
                .validate()
                .is_err()
        );
        assert!(
            with(vec!["a".into(); MAX_AUDIO_MIX_NAMES + 1])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn a_full_presence_stays_under_the_gossip_message_cap() {
        let preset = crate::messages::Preset {
            id: crate::constants::LOSSLESS_PRESET_ID,
            name: "Source lossless".into(),
            width: 3840,
            height: 2160,
            fps: 240,
            bitrate_kbps: crate::constants::MAX_BITRATE_KBPS,
            codec: crate::Codec::Hevc,
        };
        let live = |id| LiveInfo {
            id,
            title: "t".repeat(64),
            kind: crate::SourceKind::Monitor,
            source_width: 3840,
            source_height: 2160,
            source_fps: 240,
            has_audio: true,
            presets: vec![preset.clone(); MAX_PRESETS_PER_LIVE],
        };
        let full = Presence {
            version: PROTOCOL_VERSION,
            ts_unix_ms: 1_700_000_000_000,
            nickname: "n".repeat(NICKNAME_MAX_LEN),
            lives: (0..MAX_LIVES_PER_PARTICIPANT as u32).map(live).collect(),
            audio_mix: Some(AudioMix::Only {
                names: vec!["a".repeat(MAX_AUDIO_MIX_NAME_LEN); MAX_AUDIO_MIX_NAMES],
                others: u32::MAX,
            }),
            quality_presets: Vec::new(),
            voice: None,
        };
        let signed = Signed::sign(&SecretKey::from_bytes(&[3u8; 32]), &full).unwrap();
        let bytes = crate::messages::encode(&signed).unwrap();
        assert!(bytes.len() < 4096, "{} bytes", bytes.len());
    }

    #[test]
    fn signed_presence_round_trips_and_names_its_author() {
        let secret = SecretKey::from_bytes(&[3u8; 32]);
        let signed = Signed::sign(&secret, &presence("gt", 1)).unwrap();
        assert_eq!(signed.author, secret.public());
        let back: Presence = signed.verify().unwrap();
        assert_eq!(back, presence("gt", 1));
    }

    #[test]
    fn tampered_payload_fails_verification() {
        let secret = SecretKey::from_bytes(&[3u8; 32]);
        let mut signed = Signed::sign(&secret, &presence("gt", 1)).unwrap();
        signed.payload[0] ^= 0xff;
        assert!(matches!(
            signed.verify::<Presence>(),
            Err(ProtoError::BadSignature)
        ));
    }

    #[test]
    fn swapped_author_fails_verification() {
        let secret = SecretKey::from_bytes(&[3u8; 32]);
        let mut signed = Signed::sign(&secret, &presence("gt", 1)).unwrap();
        signed.author = SecretKey::from_bytes(&[4u8; 32]).public();
        assert!(matches!(
            signed.verify::<Presence>(),
            Err(ProtoError::BadSignature)
        ));
    }

    #[test]
    fn presence_validation_enforces_version_nickname_and_live_limits() {
        assert!(presence("gt", 2).validate().is_ok());
        let mut wrong_version = presence("gt", 1);
        wrong_version.version = PROTOCOL_VERSION + 1;
        assert!(matches!(
            wrong_version.validate(),
            Err(ProtoError::Invalid(_))
        ));
        assert!(matches!(
            presence(&"x".repeat(NICKNAME_MAX_LEN + 1), 1).validate(),
            Err(ProtoError::Invalid(_))
        ));
        assert!(matches!(
            presence("gt", MAX_LIVES_PER_PARTICIPANT + 1).validate(),
            Err(ProtoError::Invalid(_))
        ));
    }
}
