use iroh_base::{PublicKey, SecretKey, Signature};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};

use crate::constants::{
    MAX_AUDIO_MIX_NAME_LEN, MAX_AUDIO_MIX_NAMES, MAX_LIVES_PER_PARTICIPANT, MAX_PRESETS_PER_LIVE,
    NICKNAME_MAX_LEN, PROTOCOL_VERSION,
};
use crate::error::ProtoError;
use crate::messages::{Preset, SourceKind};

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
    /// Must stay the last field. `None` is a peer too old to say: postcard ignores trailing bytes,
    /// so such a peer reads our presence, and this read of its shorter one runs out of input here.
    #[serde(deserialize_with = "absent_when_truncated")]
    pub audio_mix: Option<AudioMix>,
}

fn absent_when_truncated<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<AudioMix>, D::Error> {
    Ok(Option::deserialize(deserializer).unwrap_or(None))
}

impl Presence {
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
        }
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
        assert_eq!(read.lives, older.lives);
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
