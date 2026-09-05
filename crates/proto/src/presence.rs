use iroh_base::{PublicKey, SecretKey, Signature};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::constants::{
    MAX_LIVES_PER_PARTICIPANT, MAX_PRESETS_PER_LIVE, NICKNAME_MAX_LEN, PROTOCOL_VERSION,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    pub version: u8,
    pub ts_unix_ms: u64,
    pub nickname: String,
    pub lives: Vec<LiveInfo>,
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
        }
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
