use crate::constants::TICKET_KIND;
use iroh_base::EndpointAddr;
use iroh_tickets::{ParseError, Ticket};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// Everything a newcomer needs to reach a room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomTicket {
    pub topic: [u8; 32],
    pub bootstrap: Vec<EndpointAddr>,
}
#[derive(Serialize, Deserialize)]
enum TicketWireFormat {
    V1(RoomTicket),
}
impl RoomTicket {
    pub fn new(topic: [u8; 32], bootstrap: Vec<EndpointAddr>) -> Self {
        Self { topic, bootstrap }
    }
    pub fn random_topic() -> [u8; 32] {
        rand::random()
    }
}
impl Ticket for RoomTicket {
    const KIND: &'static str = TICKET_KIND;
    fn encode_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(&TicketWireFormat::V1(self.clone()))
            .expect("ticket serialization is infallible")
    }
    fn decode_bytes(bytes: &[u8]) -> Result<Self, ParseError> {
        let TicketWireFormat::V1(t) = postcard::from_bytes(bytes)?;
        if t.bootstrap.is_empty() {
            return Err(ParseError::verification_failed(
                "ticket lists no bootstrap peers",
            ));
        }
        Ok(t)
    }
}
impl fmt::Display for RoomTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&Ticket::encode_string(self))
    }
}
impl FromStr for RoomTicket {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ticket::decode_string(s)
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::str::FromStr;

    use iroh_base::{EndpointAddr, SecretKey};

    use super::*;

    fn sample_addr() -> EndpointAddr {
        let id = SecretKey::from_bytes(&[7u8; 32]).public();
        EndpointAddr::new(id).with_ip_addr(SocketAddr::from(([192, 168, 1, 10], 4433)))
    }

    #[test]
    fn ticket_string_round_trips_and_carries_the_kind_prefix() {
        let ticket = RoomTicket::new([1u8; 32], vec![sample_addr()]);
        let text = ticket.to_string();
        assert!(text.starts_with("brp"), "got {text}");
        assert_eq!(RoomTicket::from_str(&text).unwrap(), ticket);
    }

    #[test]
    fn ticket_rejects_foreign_kind() {
        assert!(matches!(
            RoomTicket::from_str("endpointaaaaaaaaaaaaaaaa"),
            Err(ParseError::Kind { .. })
        ));
    }

    #[test]
    fn ticket_rejects_empty_bootstrap_list() {
        let text = RoomTicket::new([1u8; 32], vec![]).to_string();
        assert!(matches!(
            RoomTicket::from_str(&text),
            Err(ParseError::Verify { .. })
        ));
    }

    #[test]
    fn random_topics_differ() {
        assert_ne!(RoomTicket::random_topic(), RoomTicket::random_topic());
    }
}
