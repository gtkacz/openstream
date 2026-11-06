//! The pure rules of voice: what joining, muting and deafening do to you, and who hears whom.

use std::collections::HashSet;

use brp_proto::VoiceState;
use iroh::PublicKey;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct SelfVoice {
    pub joined: bool,
    pub muted: bool,
    pub deafened: bool,
    muted_before_deafen: bool,
}

impl SelfVoice {
    pub fn join(self) -> Self {
        Self {
            joined: true,
            ..Self::default()
        }
    }

    pub fn leave(self) -> Self {
        Self::default()
    }

    pub fn set_muted(self, muted: bool) -> Self {
        if !self.joined {
            return self;
        }
        // Unmuting is an intent to be heard, which a deaf listener cannot be.
        Self {
            muted,
            deafened: self.deafened && muted,
            ..self
        }
    }

    pub fn set_deafened(self, deafened: bool) -> Self {
        if !self.joined || self.deafened == deafened {
            return self;
        }
        if deafened {
            Self {
                muted: true,
                deafened: true,
                muted_before_deafen: self.muted,
                ..self
            }
        } else {
            Self {
                muted: self.muted_before_deafen,
                deafened: false,
                ..self
            }
        }
    }

    pub fn presence(self) -> Option<VoiceState> {
        self.joined.then_some(VoiceState {
            muted: self.muted,
            deafened: self.deafened,
        })
    }

    pub fn transmits(self) -> bool {
        self.joined && !self.muted
    }

    pub fn listens(self) -> bool {
        self.joined && !self.deafened
    }
}

/// The sender's test for a subscriber: it is in voice and has not deafened itself.
pub(crate) fn listens_to(presence: Option<&VoiceState>) -> bool {
    presence.is_some_and(|voice| !voice.deafened)
}

/// Everyone in voice, muted or not: muting keeps the subscription and drops packets later.
pub(crate) fn desired_listeners<'a>(
    me: SelfVoice,
    members: impl Iterator<Item = (PublicKey, Option<&'a VoiceState>)>,
) -> HashSet<PublicKey> {
    if !me.listens() {
        return HashSet::new();
    }
    members
        .filter_map(|(id, voice)| voice.map(|_| id))
        .collect()
}

#[cfg(test)]
mod tests {
    use iroh::SecretKey;

    use super::*;

    fn key(seed: u8) -> PublicKey {
        SecretKey::from_bytes(&[seed; 32]).public()
    }

    #[test]
    fn joining_starts_unmuted_and_undeafened() {
        let me = SelfVoice::default().join();
        assert!(me.joined && !me.muted && !me.deafened);
    }

    #[test]
    fn leaving_forgets_everything() {
        let me = SelfVoice::default().join().set_deafened(true).leave();
        assert_eq!(me, SelfVoice::default());
    }

    #[test]
    fn deafening_also_mutes() {
        let me = SelfVoice::default().join().set_deafened(true);
        assert!(me.muted && me.deafened);
    }

    #[test]
    fn undeafening_restores_the_mute_from_before() {
        let joined = SelfVoice::default().join();
        assert!(!joined.set_deafened(true).set_deafened(false).muted);
        let muted = joined.set_muted(true);
        assert!(muted.set_deafened(true).set_deafened(false).muted);
    }

    #[test]
    fn unmuting_while_deafened_undeafens_too() {
        let me = SelfVoice::default()
            .join()
            .set_deafened(true)
            .set_muted(false);
        assert!(!me.muted && !me.deafened);
    }

    #[test]
    fn controls_do_nothing_out_of_voice() {
        let out = SelfVoice::default();
        assert_eq!(out.set_muted(true), out);
        assert_eq!(out.set_deafened(true), out);
    }

    #[test]
    fn presence_says_nothing_out_of_voice() {
        assert_eq!(SelfVoice::default().presence(), None);
        assert_eq!(
            SelfVoice::default().join().set_muted(true).presence(),
            Some(VoiceState {
                muted: true,
                deafened: false
            })
        );
    }

    #[test]
    fn only_people_in_voice_and_hearing_are_listeners() {
        assert!(!listens_to(None));
        assert!(!listens_to(Some(&VoiceState {
            muted: true,
            deafened: true
        })));
        assert!(listens_to(Some(&VoiceState {
            muted: true,
            deafened: false
        })));
    }

    #[test]
    fn nobody_out_of_voice_is_listened_to() {
        let in_voice = VoiceState::default();
        let members = || [(key(1), Some(&in_voice)), (key(2), None)].into_iter();
        let me = SelfVoice::default().join();
        assert_eq!(desired_listeners(me, members()), HashSet::from([key(1)]));
        assert!(desired_listeners(SelfVoice::default(), members()).is_empty());
        assert!(desired_listeners(me.set_deafened(true), members()).is_empty());
    }
}
