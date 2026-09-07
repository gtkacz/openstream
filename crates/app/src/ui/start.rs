//! The start screen: nickname and ticket entry with Create and Join, shown until a room is open.

use std::str::FromStr;

use brp_proto::RoomTicket;

use crate::launch::Intent;
use crate::settings::RecentRoom;

/// Which button the user clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartAction {
    Create,
    Join,
    OpenSettings,
}

/// The form's fields, whether an open is in flight, and the last error to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartState {
    pub nickname: String,
    pub ticket: String,
    pub connecting: bool,
    pub error: String,
}

impl StartState {
    pub fn new(nickname: String) -> Self {
        Self {
            nickname,
            ticket: String::new(),
            connecting: false,
            error: String::new(),
        }
    }

    /// Turns a click into an intent, or refuses it: nothing while an open is in flight, and a join
    /// needs a ticket that parses. On success the screen is marked connecting.
    pub fn submit(&mut self, action: StartAction) -> Option<Intent> {
        if action == StartAction::OpenSettings {
            return None;
        }
        if self.connecting {
            return None;
        }
        let intent = match action {
            StartAction::Create => Intent::Create,
            StartAction::Join => match RoomTicket::from_str(self.ticket.trim()) {
                Ok(ticket) => Intent::Join(ticket),
                Err(error) => {
                    self.error = format!("invalid ticket: {error}");
                    return None;
                }
            },
            // Returned above before this match is reached.
            StartAction::OpenSettings => unreachable!(),
        };
        self.error.clear();
        self.connecting = true;
        Some(intent)
    }

    /// The open failed: back to the form with the reason shown.
    pub fn failed(&mut self, message: String) {
        self.connecting = false;
        self.error = message;
    }
}

/// Characters of a ticket shown at each end on the start screen; the middle is elided.
const TICKET_HEAD: usize = 8;
const TICKET_TAIL: usize = 6;

/// "just now", then minutes, hours, days.
pub fn relative_age(then_unix: u64, now_unix: u64) -> String {
    let seconds = now_unix.saturating_sub(then_unix);
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if minutes == 0 {
        "just now".to_string()
    } else if hours == 0 {
        format!("{minutes} min ago")
    } else if days == 0 {
        format!("{hours} h ago")
    } else {
        format!("{days} d ago")
    }
}

/// The ticket's ends with the middle elided, so a row fits and two tickets stay tellable apart.
pub fn abbreviate_ticket(ticket: &str) -> String {
    let chars: Vec<char> = ticket.chars().collect();
    if chars.len() <= TICKET_HEAD + TICKET_TAIL + 1 {
        return ticket.to_string();
    }
    let head: String = chars[..TICKET_HEAD].iter().collect();
    let tail: String = chars[chars.len() - TICKET_TAIL..].iter().collect();
    format!("{head}…{tail}")
}

/// Draws the start screen and returns the button clicked, if any. Clicking a recent room fills
/// the ticket field and joins it.
pub fn draw(
    ui: &mut egui::Ui,
    state: &mut StartState,
    recent: &[RecentRoom],
    now_unix: u64,
) -> Option<StartAction> {
    let mut action = None;
    egui::CentralPanel::default().show(ui, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            if ui.button("Settings").clicked() {
                action = Some(StartAction::OpenSettings);
            }
        });
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.2);
            ui.heading("brp");
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.label("Nickname");
                ui.add_enabled(
                    !state.connecting,
                    egui::TextEdit::singleline(&mut state.nickname).desired_width(240.0),
                );
            });
            ui.add_space(8.0);
            if ui
                .add_enabled(!state.connecting, egui::Button::new("Create room"))
                .clicked()
            {
                action = Some(StartAction::Create);
            }
            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);
            ui.add_enabled(
                !state.connecting,
                egui::TextEdit::singleline(&mut state.ticket)
                    .hint_text("paste a ticket")
                    .desired_width(480.0),
            );
            let can_join = !state.connecting && !state.ticket.trim().is_empty();
            if ui
                .add_enabled(can_join, egui::Button::new("Join room"))
                .clicked()
            {
                action = Some(StartAction::Join);
            }
            if !recent.is_empty() {
                ui.add_space(16.0);
                ui.label("Recent rooms");
                for room in recent {
                    ui.horizontal(|ui| {
                        ui.monospace(abbreviate_ticket(&room.ticket));
                        ui.weak(relative_age(room.last_joined_unix, now_unix));
                        if ui
                            .add_enabled(!state.connecting, egui::Button::new("Join"))
                            .clicked()
                        {
                            state.ticket = room.ticket.clone();
                            action = Some(StartAction::Join);
                        }
                    });
                }
            }
            ui.add_space(16.0);
            if state.connecting {
                ui.weak("connecting");
            }
            if !state.error.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &state.error);
            }
        });
    });
    action
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use iroh::{EndpointAddr, SecretKey};

    use super::*;

    fn valid_ticket() -> RoomTicket {
        let id = SecretKey::from_bytes(&[7u8; 32]).public();
        let addr = EndpointAddr::new(id).with_ip_addr(SocketAddr::from(([192, 168, 1, 10], 4433)));
        RoomTicket::new([1u8; 32], vec![addr])
    }

    #[test]
    fn create_yields_the_create_intent_and_marks_connecting() {
        let mut state = StartState::new("alice".into());
        assert_eq!(state.submit(StartAction::Create), Some(Intent::Create));
        assert!(state.connecting);
        assert!(state.error.is_empty());
    }

    #[test]
    fn join_with_garbage_shows_an_error_and_stays_on_the_form() {
        let mut state = StartState::new("alice".into());
        state.ticket = "not a ticket".into();
        assert_eq!(state.submit(StartAction::Join), None);
        assert!(!state.connecting);
        assert!(state.error.starts_with("invalid ticket"), "{}", state.error);
    }

    #[test]
    fn join_with_a_valid_ticket_yields_the_join_intent() {
        let ticket = valid_ticket();
        let mut state = StartState::new("alice".into());
        state.ticket = format!("  {ticket}\n");
        assert_eq!(state.submit(StartAction::Join), Some(Intent::Join(ticket)));
        assert!(state.connecting);
    }

    #[test]
    fn nothing_is_accepted_while_connecting() {
        let mut state = StartState::new("alice".into());
        state.submit(StartAction::Create);
        assert_eq!(state.submit(StartAction::Create), None);
        assert_eq!(state.submit(StartAction::Join), None);
    }

    #[test]
    fn open_settings_is_never_an_intent_and_leaves_the_form_alone() {
        let mut state = StartState::new("alice".into());
        assert_eq!(state.submit(StartAction::OpenSettings), None);
        assert!(!state.connecting);
    }

    #[test]
    fn relative_age_rounds_down_through_the_units() {
        assert_eq!(relative_age(1_000, 1_030), "just now");
        assert_eq!(relative_age(1_000, 1_000 + 5 * 60), "5 min ago");
        assert_eq!(relative_age(1_000, 1_000 + 3 * 3_600 + 59), "3 h ago");
        assert_eq!(relative_age(1_000, 1_000 + 2 * 86_400), "2 d ago");
        assert_eq!(relative_age(2_000, 1_000), "just now");
    }

    #[test]
    fn short_tickets_are_shown_whole_and_long_ones_keep_both_ends() {
        assert_eq!(abbreviate_ticket("brpshort"), "brpshort");
        let long = "brp".to_string() + &"x".repeat(40) + "tail42";
        let shown = abbreviate_ticket(&long);
        assert!(shown.starts_with("brpxxxxx"), "{shown}");
        assert!(shown.ends_with("tail42"), "{shown}");
        assert!(shown.contains('…'), "{shown}");
        assert_eq!(shown.chars().count(), TICKET_HEAD + TICKET_TAIL + 1);
    }

    #[test]
    fn a_failure_returns_to_the_form_with_the_message() {
        let mut state = StartState::new("alice".into());
        state.submit(StartAction::Create);
        state.failed("no room member answered within the join timeout".into());
        assert!(!state.connecting);
        assert_eq!(
            state.error,
            "no room member answered within the join timeout"
        );
        assert_eq!(state.submit(StartAction::Create), Some(Intent::Create));
    }
}
