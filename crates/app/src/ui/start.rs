//! The start screen: nickname and ticket entry with Create and Join, shown until a room is open.

use brp_net::PathKind;

use super::members::{self, PathShape};
use super::motion;
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
const FORM_WIDTH: f32 = 380.0;
const COLUMN_GAP: f32 = 64.0;
/// The brand, the form, the diagram and the settings button all share this column, so on a very wide
/// window they stay on two common edges instead of drifting to the corners of the screen.
const CONTENT_WIDTH: f32 = 1200.0;

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
    super::motion::dissolve_in(ui.ctx(), "start", theme::GROUND);
    egui::CentralPanel::default()
        .frame(
            egui::Frame::NONE
                .fill(theme::GROUND)
                .inner_margin(egui::Margin::same(24)),
        )
        .show(ui, |ui| {
            let space = ui.max_rect();
            let column = egui::Rect::from_center_size(
                space.center(),
                egui::vec2(space.width().min(CONTENT_WIDTH), space.height()),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(column), |ui| {
                theme::split_row(ui, theme::CONTROL_HEIGHT, theme::brand, |ui| {
                    if theme::icon_button(ui, icons::SETTINGS, "Settings", true).clicked() {
                        action = Some(StartAction::OpenSettings);
                    }
                });
                let body = ui.available_rect_before_wrap();
                let with_diagram = ui.ctx().content_rect().width() >= DIAGRAM_MIN_WINDOW_WIDTH;
                if with_diagram {
                    let beside = egui::pos2(body.left() + FORM_WIDTH + COLUMN_GAP, body.top());
                    room_diagram(ui, egui::Rect::from_min_max(beside, body.max), state);
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let width = ui.available_width().min(FORM_WIDTH);
                    let inset = if with_diagram {
                        0.0
                    } else {
                        ((ui.available_width() - width) / 2.0).max(0.0)
                    };
                    theme::vertically_centred(ui, "start-form", |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(inset);
                            ui.vertical(|ui| {
                                ui.set_width(width);
                                form(ui, state, recent, now_unix, update_state, &mut action);
                            });
                        });
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
            .font(theme::semibold(30.0))
            .extra_letter_spacing(-0.5),
    );
    ui.label(
        egui::RichText::new("A room for your screens, windows, and audio")
            .size(16.0)
            .color(theme::SECONDARY),
    );
    ui.add_space(24.0);
    theme::section_heading(ui, "Your name");
    ui.add_enabled_ui(!state.connecting, |ui| {
        theme::field(
            ui,
            egui::TextEdit::singleline(&mut state.nickname).hint_text("Nickname"),
            theme::FORM_HEIGHT,
        );
    });
    let has_ticket = !state.ticket.trim().is_empty();
    let full_width = egui::vec2(ui.available_width(), theme::FORM_HEIGHT);
    // Pasting a ticket says what this person came to do, so Join takes the accent surface from
    // Create for as long as the field holds one. The two cross-fade.
    let (create_tone, join_tone) = if has_ticket {
        (theme::Tone::Neutral, theme::Tone::Primary)
    } else {
        (theme::Tone::Primary, theme::Tone::Neutral)
    };
    if theme::wide_action(
        ui,
        icons::CREATE,
        "Create room",
        create_tone,
        !state.connecting,
        full_width,
    )
    .clicked()
    {
        *action = Some(StartAction::Create);
    }
    ui.add_space(24.0);
    theme::section_heading(ui, "Have an invite?");
    ui.add_enabled_ui(!state.connecting, |ui| {
        theme::field(
            ui,
            egui::TextEdit::singleline(&mut state.ticket).hint_text("Paste a room link or ticket"),
            theme::FORM_HEIGHT,
        );
    });
    let can_join = !state.connecting && has_ticket;
    if theme::wide_action(
        ui,
        icons::JOIN,
        "Join room",
        join_tone,
        can_join,
        full_width,
    )
    .clicked()
    {
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
            theme::split_row(
                ui,
                theme::CONTROL_HEIGHT,
                |ui| {
                    ui.label(theme::ticket_text(abbreviate_ticket(&room.ticket)));
                },
                |ui| {
                    if theme::text_button(ui, "Join", !state.connecting).clicked() {
                        state.ticket = room.ticket.clone();
                        *action = Some(StartAction::Join);
                    }
                    ui.weak(relative_age(room.last_joined_unix, now_unix));
                },
            );
        }
    }
    if update_state.available.is_some() {
        ui.add_space(16.0);
        if update::draw(ui, update_state) {
            *action = Some(StartAction::Update);
        }
    }
}

/// Where the diagram's three screens sit in `area`: the sharer across the full width, and two
/// viewers beneath it that end on its edges, so the figure has one axis and two clean sides. It ends
/// on the right of `area`, sharing an edge with the settings button above it.
fn diagram_screens(area: egui::Rect) -> [egui::Rect; 3] {
    const ASPECT: f32 = 9.0 / 16.0;
    const LINK_HEIGHT: f32 = 72.0;
    const VIEWER_GAP: f32 = 16.0;
    const WIDEST: f32 = 600.0;
    // Height = width * ASPECT + LINK_HEIGHT + (width - VIEWER_GAP) / 2 * ASPECT, solved for width.
    let fits_height = (area.height() - LINK_HEIGHT + VIEWER_GAP / 2.0 * ASPECT) / (1.5 * ASPECT);
    let width = area.width().min(fits_height).clamp(VIEWER_GAP, WIDEST);
    let viewer = egui::vec2(
        (width - VIEWER_GAP) / 2.0,
        (width - VIEWER_GAP) / 2.0 * ASPECT,
    );
    let total_height = width * ASPECT + LINK_HEIGHT + viewer.y;
    let left = area.right() - width;
    let top = area.center().y - total_height / 2.0;
    let sharer =
        egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(width, width * ASPECT));
    let viewers_top = sharer.bottom() + LINK_HEIGHT;
    [
        sharer,
        egui::Rect::from_min_size(egui::pos2(left, viewers_top), viewer),
        egui::Rect::from_min_size(egui::pos2(area.right() - viewer.x, viewers_top), viewer),
    ]
}

/// What a screen in the diagram says, moving from `away` to `here` as `blend` rises: the words
/// cross-fade and the chip's width follows, so a name passing between screens never jumps.
struct DiagramChip {
    away: String,
    here: String,
    blend: f32,
    ink: egui::Color32,
    dot: egui::Color32,
}

fn paint_chip(painter: &egui::Painter, rect: egui::Rect, chip: DiagramChip) {
    const DOT_SPACE: f32 = 20.0;
    const SIDES: egui::Vec2 = egui::vec2(28.0, 8.0);
    let widest = (rect.width() - 20.0 - SIDES.x).max(0.0);
    let galley = |text: String| {
        let mut job = egui::text::LayoutJob::simple_singleline(text, theme::medium(12.0), chip.ink);
        job.wrap = egui::text::TextWrapping::truncate_at_width(widest);
        painter.layout_job(job)
    };
    let (away, here) = (galley(chip.away), galley(chip.here));
    let size = egui::vec2(
        egui::lerp(away.size().x..=here.size().x, chip.blend),
        away.size().y,
    ) + SIDES;
    let frame = egui::Rect::from_min_size(rect.min + egui::Vec2::splat(10.0), size);
    painter.rect_filled(frame, theme::RADIUS_CHIP, theme::SURFACE);
    painter.circle_filled(frame.left_center() + egui::vec2(11.0, 0.0), 3.0, chip.dot);
    let at = frame.min + egui::vec2(DOT_SPACE, 4.0);
    let words = painter.with_clip_rect(frame.intersect(painter.clip_rect()));
    if chip.blend < 1.0 {
        words.galley(at, away, chip.ink.gamma_multiply(1.0 - chip.blend));
    }
    if chip.blend > 0.0 {
        words.galley(at, here, chip.ink.gamma_multiply(chip.blend));
    }
}

/// What a room is, drawn with the product's own vocabulary: one person's screen arriving as the
/// same picture on everyone else's, with no server in the middle, directly where it can and through
/// a relay where it cannot. It answers the form: the name being typed is on the screen that person
/// will have, which is the sharer's until an invite in the form makes them a viewer.
fn room_diagram(ui: &egui::Ui, area: egui::Rect, state: &StartState) {
    const DOT_RADIUS: f32 = 3.0;
    const LINK_INSET: f32 = 10.0;
    const YOU_RING: f32 = 1.5;
    let painter = ui.painter_at(area);
    let [sharer, direct_peer, relayed_peer] = diagram_screens(area);
    for (rect, path, name) in [
        (direct_peer, PathKind::Direct, "Direct"),
        (relayed_peer, PathKind::Relayed, "Relayed"),
    ] {
        let x = rect.center().x;
        let (from, to) = (
            egui::pos2(x, sharer.bottom() + LINK_INSET),
            egui::pos2(x, rect.top() - LINK_INSET),
        );
        members::paint_path(
            &painter,
            from,
            to,
            PathShape::of(path),
            DOT_RADIUS,
            theme::SECONDARY,
        );
        painter.text(
            egui::pos2(x + 14.0, (from.y + to.y) / 2.0),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(12.0),
            theme::SECONDARY,
        );
    }
    let joining = motion::follow(
        ui.ctx(),
        egui::Id::new("diagram-joining"),
        !state.ticket.trim().is_empty(),
        motion::SURFACE,
    );
    let name = state.nickname.trim();
    let with_name = |role: &str| {
        if name.is_empty() {
            role.to_string()
        } else {
            format!("{name} · {role}")
        }
    };
    let screens = [
        (
            sharer,
            1.0 - joining,
            DiagramChip {
                away: "Live".to_string(),
                here: with_name("Live"),
                blend: 1.0 - joining,
                ink: theme::LIVE,
                dot: theme::LIVE,
            },
        ),
        (
            direct_peer,
            joining,
            DiagramChip {
                away: "Watching".to_string(),
                here: with_name("Watching"),
                blend: joining,
                ink: theme::TEXT,
                dot: theme::MUTED,
            },
        ),
        (
            relayed_peer,
            0.0,
            DiagramChip {
                away: "Watching".to_string(),
                here: "Watching".to_string(),
                blend: 0.0,
                ink: theme::TEXT,
                dot: theme::MUTED,
            },
        ),
    ];
    for (rect, yours, chip) in screens {
        shared_picture(&painter, rect);
        paint_chip(&painter, rect, chip);
        if yours > 0.0 {
            painter.rect_stroke(
                rect,
                theme::RADIUS_CARD,
                egui::Stroke::new(YOU_RING, theme::ACCENT.gamma_multiply(yours)),
                egui::StrokeKind::Inside,
            );
        }
    }
}

/// The picture being shared: OpenStream's own mark, the same frame on every screen, so each peer
/// visibly holds what the sharer sends.
fn shared_picture(painter: &egui::Painter, rect: egui::Rect) {
    const PICTURE: egui::Color32 = egui::Color32::from_rgb(12, 44, 68);
    const MARK_TO_HEIGHT: f32 = 0.36;
    painter.rect_filled(rect, theme::RADIUS_CARD, PICTURE);
    crate::icon::paint_mark_at(
        painter,
        egui::Rect::from_center_size(
            rect.center(),
            egui::Vec2::splat(rect.height() * MARK_TO_HEIGHT),
        ),
    );
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
    fn the_diagram_keeps_one_axis_and_two_clean_sides_at_every_window_size() {
        for (width, height) in [
            (468.0, 516.0),
            (788.0, 716.0),
            (1200.0, 1000.0),
            (500.0, 300.0),
        ] {
            let area =
                egui::Rect::from_min_size(egui::pos2(444.0, 60.0), egui::vec2(width, height));
            let [sharer, left, right] = diagram_screens(area);
            assert!(
                (left.left() - sharer.left()).abs() < 0.01
                    && (right.right() - sharer.right()).abs() < 0.01,
                "viewers must end on the sharer's edges at {width} by {height}"
            );
            assert_eq!(left.size(), right.size());
            assert_eq!(left.top(), right.top());
            assert!(
                (sharer.right() - area.right()).abs() < 0.01,
                "ends on the column's edge"
            );
            assert!(
                ((sharer.top() + left.bottom()) / 2.0 - area.center().y).abs() < 0.01,
                "centred on the area's height"
            );
            assert!(
                sharer.top() >= area.top() - 0.01 && left.bottom() <= area.bottom() + 0.01,
                "{width} by {height} overflows: {sharer:?} {left:?}"
            );
        }
    }

    #[test]
    fn a_diagram_with_no_room_collapses_instead_of_inverting() {
        for (width, height) in [(10.0, 500.0), (500.0, 40.0), (0.0, 0.0)] {
            let area =
                egui::Rect::from_min_size(egui::pos2(444.0, 60.0), egui::vec2(width, height));
            for screen in diagram_screens(area) {
                assert!(
                    screen.width() >= 0.0 && screen.height() >= 0.0,
                    "{width} by {height} gave {screen:?}"
                );
            }
        }
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
