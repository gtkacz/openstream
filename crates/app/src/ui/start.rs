//! The start screen: nickname and ticket entry with Create and Join, shown until a room is open.

use super::theme::{self, icons};
use super::update::{self, UpdateState};
use crate::launch::Intent;
use crate::link;
use crate::settings::RecentRoom;

/// Which button the user clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartAction {
    Create,
    Join,
    OpenSettings,
    Update,
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
        if matches!(action, StartAction::OpenSettings | StartAction::Update) {
            return None;
        }
        if self.connecting {
            return None;
        }
        let intent = match action {
            StartAction::Create => Intent::Create,
            StartAction::Join => match link::parse_ticket(&self.ticket) {
                Ok(ticket) => Intent::Join(ticket),
                Err(error) => {
                    self.error = format!("invalid ticket: {error}");
                    return None;
                }
            },
            // Returned above before this match is reached.
            StartAction::OpenSettings | StartAction::Update => unreachable!(),
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

    /// Adds a message to the error line. An earlier message stays on its own line: the settings
    /// load error and a bad link can both be true at launch, and neither should hide the other.
    pub fn show_error(&mut self, message: &str) {
        if !self.error.is_empty() {
            self.error.push('\n');
        }
        self.error.push_str(message);
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

/// Wide enough to set the room diagram beside the form instead of dropping it.
const DIAGRAM_MIN_WINDOW_WIDTH: f32 = 960.0;
const FORM_WIDTH: f32 = 420.0;

/// Draws the start screen and returns the button clicked, if any. Clicking a recent room fills
/// the ticket field and joins it.
pub fn draw(
    ui: &mut egui::Ui,
    state: &mut StartState,
    recent: &[RecentRoom],
    now_unix: u64,
    update_state: &UpdateState,
) -> Option<StartAction> {
    let mut action = None;
    egui::CentralPanel::default()
        .frame(
            egui::Frame::NONE
                .fill(theme::GROUND)
                .inner_margin(egui::Margin::same(24)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                theme::brand(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::icon_button(ui, icons::SETTINGS, "Settings", true).clicked() {
                        action = Some(StartAction::OpenSettings);
                    }
                });
            });
            let body = ui.available_rect_before_wrap();
            let with_diagram = ui.ctx().content_rect().width() >= DIAGRAM_MIN_WINDOW_WIDTH;
            if with_diagram {
                let diagram = egui::Rect::from_min_max(
                    egui::pos2(body.left() + FORM_WIDTH + 72.0, body.top() + 24.0),
                    body.max,
                );
                room_diagram(ui, diagram);
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                let width = ui.available_width().min(FORM_WIDTH);
                let inset = if with_diagram {
                    0.0
                } else {
                    ((ui.available_width() - width) / 2.0).max(0.0)
                };
                ui.add_space((ui.available_height() * 0.1).clamp(24.0, 88.0));
                ui.horizontal(|ui| {
                    ui.add_space(inset);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        form(ui, state, recent, now_unix, update_state, &mut action);
                    });
                });
            });
        });
    action
}

fn form(
    ui: &mut egui::Ui,
    state: &mut StartState,
    recent: &[RecentRoom],
    now_unix: u64,
    update_state: &UpdateState,
    action: &mut Option<StartAction>,
) {
    ui.label(
        egui::RichText::new("Share your screen")
            .font(theme::semibold(36.0))
            .extra_letter_spacing(-0.5),
    );
    ui.label(
        egui::RichText::new("A room for your screens, windows, and audio")
            .size(16.0)
            .color(theme::SECONDARY),
    );
    ui.add_space(24.0);
    theme::section_heading(ui, "Your name");
    ui.add_enabled(
        !state.connecting,
        egui::TextEdit::singleline(&mut state.nickname)
            .hint_text("Nickname")
            .margin(egui::vec2(12.0, 10.0))
            .desired_width(f32::INFINITY),
    );
    ui.add_space(16.0);
    let has_ticket = !state.ticket.trim().is_empty();
    let full_width = egui::vec2(ui.available_width(), 40.0);
    // Pasting a ticket says what this person came to do, so Join takes the light surface from
    // Create for as long as the field holds one.
    let create =
        egui::Button::new((theme::icon(icons::CREATE), "Create room")).min_size(full_width);
    let created = if has_ticket {
        ui.add_enabled(!state.connecting, create)
    } else {
        theme::inverted(ui, |ui| ui.add_enabled(!state.connecting, create))
    };
    if created.clicked() {
        *action = Some(StartAction::Create);
    }
    ui.add_space(20.0);
    theme::section_heading(ui, "Have an invite?");
    ui.add_enabled(
        !state.connecting,
        egui::TextEdit::singleline(&mut state.ticket)
            .hint_text("Paste a room link or ticket")
            .margin(egui::vec2(12.0, 10.0))
            .desired_width(f32::INFINITY),
    );
    let can_join = !state.connecting && has_ticket;
    let join = egui::Button::new((theme::icon(icons::JOIN), "Join room")).min_size(full_width);
    let joined = if has_ticket {
        theme::inverted(ui, |ui| ui.add_enabled(can_join, join))
    } else {
        ui.add_enabled(can_join, join)
    };
    if joined.clicked() {
        *action = Some(StartAction::Join);
    }
    if state.connecting {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            theme::spinner(ui, 16.0);
            ui.weak("Connecting to the room…");
        });
    }
    if !state.error.is_empty() {
        ui.add_space(8.0);
        ui.colored_label(theme::ERROR, &state.error);
    }
    if !recent.is_empty() {
        ui.add_space(24.0);
        theme::section_heading(ui, "Recent rooms");
        for room in recent {
            ui.horizontal(|ui| {
                ui.label(theme::figure(abbreviate_ticket(&room.ticket)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!state.connecting, egui::Button::new("Join"))
                        .clicked()
                    {
                        state.ticket = room.ticket.clone();
                        *action = Some(StartAction::Join);
                    }
                    ui.weak(relative_age(room.last_joined_unix, now_unix));
                });
            });
        }
    }
    ui.add_space(16.0);
    if update::draw(ui, update_state) {
        *action = Some(StartAction::Update);
    }
}

/// What a room is, drawn with the product's own vocabulary: one person's screen arriving as the
/// same picture on everyone else's, with no server in the middle, directly where it can and through
/// a relay where it cannot. Fractions of the area, so it scales with the window.
fn room_diagram(ui: &egui::Ui, area: egui::Rect) {
    const SCREENS: [(f32, f32, f32); 3] =
        [(0.44, 0.31, 0.74), (0.80, 0.77, 0.40), (0.17, 0.80, 0.30)];
    const ASPECT: f32 = 9.0 / 16.0;
    let scale = area.width().min(area.height() * 1.1);
    let painter = ui.painter_at(area);
    let point = |x: f32, y: f32| area.lerp_inside(egui::vec2(x, y));
    let [sharer, direct_peer, relayed_peer] = SCREENS.map(|(x, y, _)| point(x, y));
    let rule = egui::Stroke::new(1.0, theme::MUTED);
    painter.line_segment([sharer, direct_peer], rule);
    painter.line_segment([sharer, relayed_peer], rule);
    painter.extend(egui::Shape::dashed_line(
        &[direct_peer, relayed_peer],
        rule,
        4.0,
        4.0,
    ));
    for (index, (x, y, width)) in SCREENS.into_iter().enumerate() {
        let rect =
            egui::Rect::from_center_size(point(x, y), egui::vec2(width, width * ASPECT) * scale);
        shared_picture(&painter, rect);
        // The same ring that rounds real tiles rounds these.
        painter.rect_stroke(
            rect,
            10.0,
            egui::Stroke::new(6.0, theme::GROUND),
            egui::StrokeKind::Outside,
        );
        let is_sharer = index == 0;
        let (text, ink) = if is_sharer {
            ("Live", theme::LIVE)
        } else {
            ("Watching", theme::TEXT)
        };
        let galley = painter.layout_no_wrap(text.to_string(), theme::medium(12.0), ink);
        let chip = egui::Rect::from_min_size(
            rect.min + egui::vec2(10.0, 10.0),
            galley.size() + egui::vec2(28.0, 8.0),
        );
        painter.rect_filled(chip, 6.0, theme::SURFACE);
        painter.circle_filled(
            chip.left_center() + egui::vec2(11.0, 0.0),
            3.0,
            if is_sharer { theme::LIVE } else { theme::MUTED },
        );
        painter.galley(chip.min + egui::vec2(20.0, 4.0), galley, ink);
    }
    let tag = |at: egui::Pos2, text: &str| {
        let galley = painter.layout_no_wrap(
            text.to_string(),
            egui::FontId::proportional(12.0),
            theme::SECONDARY,
        );
        let backing = egui::Rect::from_center_size(at, galley.size() + egui::vec2(12.0, 6.0));
        painter.rect_filled(backing, 4.0, theme::GROUND);
        painter.galley(at - galley.size() / 2.0, galley, theme::SECONDARY);
    };
    // Further along than halfway on the sharer's links: its large screen covers their first half.
    tag(sharer + (direct_peer - sharer) * 0.62, "Direct");
    tag(sharer + (relayed_peer - sharer) * 0.62, "Direct");
    tag(direct_peer + (relayed_peer - direct_peer) * 0.5, "Relayed");
}

/// The picture being shared: a low sun going down behind a ridgeline, in flat fills from the live
/// hue's family. Drawn once per screen, so every peer visibly holds the same frame.
fn shared_picture(painter: &egui::Painter, rect: egui::Rect) {
    const SKY: egui::Color32 = egui::Color32::from_rgb(13, 44, 45);
    const FAR: egui::Color32 = egui::Color32::from_rgb(9, 30, 33);
    const NEAR: egui::Color32 = egui::Color32::from_rgb(6, 18, 21);
    const SUN_CENTER: (f32, f32) = (0.63, 0.56);
    const SUN_RADIUS: f32 = 0.36;
    /// Slices taken out of the sun's lower half: top of each as a fraction of the height, then its
    /// thickness.
    const SUN_BANDS: [(f32, f32); 3] = [(0.50, 0.02), (0.56, 0.03), (0.63, 0.045)];
    const FAR_RIDGE: [(f32, f32); 7] = [
        (0.0, 0.70),
        (0.16, 0.54),
        (0.30, 0.66),
        (0.46, 0.60),
        (0.68, 0.74),
        (0.86, 0.58),
        (1.0, 0.66),
    ];
    const NEAR_RIDGE: [(f32, f32); 6] = [
        (0.0, 0.86),
        (0.20, 0.72),
        (0.42, 0.88),
        (0.64, 0.80),
        (0.82, 0.90),
        (1.0, 0.76),
    ];
    let at = |x: f32, y: f32| rect.lerp_inside(egui::vec2(x, y));
    painter.rect_filled(rect, 0.0, SKY);
    painter.circle_filled(
        at(SUN_CENTER.0, SUN_CENTER.1),
        rect.height() * SUN_RADIUS,
        theme::LIVE,
    );
    for (top, thickness) in SUN_BANDS {
        painter.rect_filled(
            egui::Rect::from_min_max(at(0.0, top), at(1.0, top + thickness)),
            0.0,
            SKY,
        );
    }
    for (ridge, fill) in [(&FAR_RIDGE[..], FAR), (&NEAR_RIDGE[..], NEAR)] {
        // A ridgeline as one polygon is concave and epaint only fills convex shapes, so each span
        // is its own quad down to the bottom edge. Each runs a point under the next, because two
        // feathered edges meeting exactly would leave a visible seam.
        for span in ridge.windows(2) {
            let ((x0, y0), (x1, y1)) = (span[0], span[1]);
            let overlap = egui::vec2(1.0, 0.0);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    at(x0, y0),
                    at(x1, y1) + overlap,
                    at(x1, 1.0) + overlap,
                    at(x0, 1.0),
                ],
                fill,
                egui::Stroke::NONE,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use brp_proto::RoomTicket;
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
    fn join_with_a_share_link_or_scheme_link_yields_the_join_intent() {
        let ticket = valid_ticket();
        for pasted in [
            format!("https://gtkacz.github.io/openstream/join/#{ticket}"),
            format!("brp://join/{ticket}"),
        ] {
            let mut state = StartState::new("alice".into());
            state.ticket = pasted.clone();
            assert_eq!(
                state.submit(StartAction::Join),
                Some(Intent::Join(ticket.clone())),
                "{pasted}"
            );
        }
    }

    #[test]
    fn nothing_is_accepted_while_connecting() {
        let mut state = StartState::new("alice".into());
        state.submit(StartAction::Create);
        assert_eq!(state.submit(StartAction::Create), None);
        assert_eq!(state.submit(StartAction::Join), None);
    }

    #[test]
    fn open_settings_and_update_are_never_intents_and_leave_the_form_alone() {
        let mut state = StartState::new("alice".into());
        assert_eq!(state.submit(StartAction::OpenSettings), None);
        assert_eq!(state.submit(StartAction::Update), None);
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

    #[test]
    fn show_error_keeps_an_earlier_message_on_its_own_line() {
        let mut state = StartState::new("alice".into());
        state.show_error("settings not loaded");
        assert_eq!(state.error, "settings not loaded");
        state.show_error("invalid ticket: bad kind");
        assert_eq!(state.error, "settings not loaded\ninvalid ticket: bad kind");
    }
}
