//! Join links: the share link the room rail copies, and the three forms every ticket entry
//! point accepts: a raw ticket, the `brp://` URL the operating system hands the binary, and the
//! share link itself.

use std::str::FromStr;

use brp_proto::RoomTicket;
use iroh_tickets::ParseError;

/// The static page that forwards a share link to the `brp://` handler.
pub const JOIN_PAGE: &str = "https://gtkacz.github.io/openstream/join/";
/// What the page hands the operating system, and so what the binary receives from a click.
pub const SCHEME_JOIN_PREFIX: &str = "brp://join/";

/// Where a link the system hands a running window should open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkRoute {
    /// In this window, which is idle on its start screen.
    Here,
    /// In a new `brp join` process, leaving this window's room alone: what a click does on
    /// Linux and Windows, where every link starts a process.
    NewWindow,
}

/// Where a link should open given this window's state: here only when it shows the start screen
/// with nothing opening.
pub fn route_link(on_start_screen: bool, opening: bool) -> LinkRoute {
    if on_start_screen && !opening {
        LinkRoute::Here
    } else {
        LinkRoute::NewWindow
    }
}

/// The command line of the process that opens `link` in a new window.
pub fn new_window_args(link: &str) -> Vec<std::ffi::OsString> {
    vec!["join".into(), link.into()]
}

/// The share link for a ticket: the join page with the ticket in the fragment, which browsers
/// never send to the server.
pub fn share_link(ticket: &str) -> String {
    format!("{JOIN_PAGE}#{ticket}")
}

/// A raw ticket, a scheme link, or a share link, whitespace-trimmed, to a ticket. Only the known
/// prefixes are stripped; whatever remains is judged by the ticket parser, so an empty or foreign
/// remainder fails exactly as a bad pasted ticket does.
pub fn parse_ticket(input: &str) -> Result<RoomTicket, ParseError> {
    RoomTicket::from_str(bare_ticket(input.trim()))
}

/// Strips a share-link or scheme-link prefix; anything else is returned as is.
fn bare_ticket(input: &str) -> &str {
    // Both `…/join/#t` and `…/join#t` are accepted: a redirect may drop the trailing slash.
    if let Some(rest) = input.strip_prefix(JOIN_PAGE.trim_end_matches('/')) {
        return rest.split_once('#').map_or("", |(_, fragment)| fragment);
    }
    if let Some(rest) = input.strip_prefix(SCHEME_JOIN_PREFIX) {
        return rest.trim_end_matches('/');
    }
    input
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use iroh::{EndpointAddr, SecretKey};

    use super::*;

    fn ticket() -> RoomTicket {
        let id = SecretKey::from_bytes(&[7u8; 32]).public();
        let addr = EndpointAddr::new(id).with_ip_addr(SocketAddr::from(([192, 168, 1, 10], 4433)));
        RoomTicket::new([1u8; 32], vec![addr])
    }

    #[test]
    fn a_link_opens_here_only_on_an_idle_start_screen() {
        assert_eq!(route_link(true, false), LinkRoute::Here);
        assert_eq!(
            route_link(true, true),
            LinkRoute::NewWindow,
            "a room is already opening in this window"
        );
        assert_eq!(
            route_link(false, false),
            LinkRoute::NewWindow,
            "a room is already open in this window"
        );
    }

    #[test]
    fn a_new_window_joins_the_link_it_was_given() {
        assert_eq!(
            new_window_args("brp://join/abc"),
            vec![std::ffi::OsString::from("join"), "brp://join/abc".into()]
        );
    }

    #[test]
    fn every_link_form_parses_to_the_same_ticket() {
        let expected = ticket();
        let text = expected.to_string();
        for input in [
            text.clone(),
            format!("  {text}\n"),
            format!("brp://join/{text}"),
            format!("brp://join/{text}/"),
            format!("https://gtkacz.github.io/openstream/join/#{text}"),
            format!("https://gtkacz.github.io/openstream/join#{text}"),
        ] {
            assert_eq!(parse_ticket(&input).unwrap(), expected, "{input}");
        }
    }

    #[test]
    fn links_without_a_ticket_fail_like_a_bad_ticket() {
        for input in [
            "",
            "not a ticket",
            "brp://join/",
            "https://gtkacz.github.io/openstream/join/",
            "https://gtkacz.github.io/openstream/join/#",
            "https://example.com/#brpaaaa",
        ] {
            assert!(parse_ticket(input).is_err(), "{input}");
        }
    }

    #[test]
    fn the_share_link_round_trips_through_the_parser() {
        let expected = ticket();
        let link = share_link(&expected.to_string());
        assert!(
            link.starts_with("https://gtkacz.github.io/openstream/join/#brp"),
            "{link}"
        );
        assert_eq!(parse_ticket(&link).unwrap(), expected);
    }
}
