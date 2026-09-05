//! Who is in the room, derived from verified presence messages. The gossip overlay's neighbour
//! events describe the transport, not the room, so they never touch this state.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use brp_proto::Presence;
use iroh::PublicKey;

#[derive(Debug, Clone)]
pub struct Member {
    pub id: PublicKey,
    pub presence: Presence,
    pub last_seen: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    Inserted,
    Updated,
    Refreshed,
    Stale,
}

pub struct Membership {
    members: HashMap<PublicKey, Member>,
    expiry: Duration,
}

impl Membership {
    pub fn new(expiry: Duration) -> Self {
        Self {
            members: HashMap::new(),
            expiry,
        }
    }

    /// The caller has already verified the signature and validated the presence.
    pub fn apply(&mut self, author: PublicKey, presence: Presence, now: Instant) -> Applied {
        match self.members.get_mut(&author) {
            None => {
                self.members.insert(
                    author,
                    Member {
                        id: author,
                        presence,
                        last_seen: now,
                    },
                );
                Applied::Inserted
            }
            Some(existing) if presence.ts_unix_ms <= existing.presence.ts_unix_ms => Applied::Stale,
            Some(existing) => {
                let changed = existing.presence.nickname != presence.nickname
                    || existing.presence.lives != presence.lives;
                existing.presence = presence;
                existing.last_seen = now;
                if changed {
                    Applied::Updated
                } else {
                    Applied::Refreshed
                }
            }
        }
    }

    pub fn expire(&mut self, now: Instant) -> Vec<PublicKey> {
        let expiry = self.expiry;
        let expired: Vec<PublicKey> = self
            .members
            .values()
            .filter(|m| now.duration_since(m.last_seen) >= expiry)
            .map(|m| m.id)
            .collect();
        for id in &expired {
            self.members.remove(id);
        }
        expired
    }

    pub fn is_member(&self, id: &PublicKey) -> bool {
        self.members.contains_key(id)
    }

    pub fn get(&self, id: &PublicKey) -> Option<&Member> {
        self.members.get(id)
    }

    pub fn members(&self) -> impl Iterator<Item = &Member> {
        self.members.values()
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use brp_proto::constants::PROTOCOL_VERSION;
    use iroh::SecretKey;

    use super::*;

    fn presence(ts: u64, nickname: &str) -> Presence {
        Presence {
            version: PROTOCOL_VERSION,
            ts_unix_ms: ts,
            nickname: nickname.into(),
            lives: vec![],
        }
    }

    fn key(seed: u8) -> PublicKey {
        SecretKey::from_bytes(&[seed; 32]).public()
    }

    #[test]
    fn insert_update_refresh_and_stale_are_told_apart() {
        let mut m = Membership::new(Duration::from_secs(20));
        let t = Instant::now();
        assert_eq!(m.apply(key(1), presence(10, "a"), t), Applied::Inserted);
        assert_eq!(m.apply(key(1), presence(11, "a"), t), Applied::Refreshed);
        assert_eq!(m.apply(key(1), presence(12, "b"), t), Applied::Updated);
        assert_eq!(
            m.apply(key(1), presence(12, "c"), t),
            Applied::Stale,
            "equal timestamp is not newer"
        );
        assert_eq!(m.apply(key(1), presence(5, "c"), t), Applied::Stale);
        assert_eq!(m.get(&key(1)).unwrap().presence.nickname, "b");
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn silent_members_expire_and_others_stay() {
        let mut m = Membership::new(Duration::from_secs(20));
        let t = Instant::now();
        m.apply(key(1), presence(1, "a"), t);
        m.apply(key(2), presence(1, "b"), t + Duration::from_secs(15));
        assert_eq!(m.expire(t + Duration::from_secs(19)), vec![]);
        assert_eq!(m.expire(t + Duration::from_secs(20)), vec![key(1)]);
        assert!(!m.is_member(&key(1)) && m.is_member(&key(2)));
    }

    #[test]
    fn refresh_updates_last_seen() {
        let mut m = Membership::new(Duration::from_secs(20));
        let t = Instant::now();
        m.apply(key(1), presence(1, "a"), t);
        m.apply(key(1), presence(2, "a"), t + Duration::from_secs(19));
        assert!(m.expire(t + Duration::from_secs(21)).is_empty());
    }
}
