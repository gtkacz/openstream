//! The start screen: nickname and ticket entry with Create and Join, shown until a room is open.

use super::theme::{self, icons};
use super::update::{self, UpdateState};
use super::wall;
use crate::launch::Intent;
use crate::link;
use crate::render::backdrop::{self, BackdropKind, BackdropRequest};
use crate::settings::RecentRoom;

/// Which button the user clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartAction {
    Create,
    Join,
    /// The answer to an invite was no.
    Decline,
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
    /// A link asked to open the room in `ticket`, and the person has not answered yet.
    pub invited: bool,
    /// Join takes the keyboard on an invite's first pass, so Enter answers it.
    focus_join: bool,
}

impl StartState {
    pub fn new(nickname: String) -> Self {
        Self {
            nickname,
            ticket: String::new(),
            connecting: false,
            error: String::new(),
            invited: false,
            focus_join: false,
        }
    }

    /// A link asked to open this room. Any web page can fire one, so the window asks first.
    pub fn invite(&mut self, ticket: String) {
        self.ticket = ticket;
        self.invited = true;
        self.focus_join = true;
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
        if action == StartAction::Decline {
            self.ticket.clear();
            self.invited = false;
            self.error.clear();
            return None;
        }
        let intent = match action {
            StartAction::Create => Intent::Create,
            StartAction::Join => match link::parse_ticket(&self.ticket) {
                Ok(ticket) => Intent::Join(ticket),
                Err(error) => {
                    self.error = crate::error::AppError::Ticket(error).to_string();
                    return None;
                }
            },
            // Returned above before this match is reached.
            StartAction::Decline | StartAction::OpenSettings | StartAction::Update => {
                unreachable!()
            }
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

/// Wide enough to set the wall beside the form instead of dropping it.
const WALL_MIN_WINDOW_WIDTH: f32 = 960.0;
const FORM_WIDTH: f32 = 380.0;
const COLUMN_GAP: f32 = 64.0;
/// The brand, the form and the settings button all share this column, so on a very wide
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
    let screen = if state.invited {
        "start-invite"
    } else {
        "start"
    };
    super::motion::dissolve_in(ui.ctx(), screen, theme::ground());
    backdrop::request(
        ui.ctx(),
        BackdropRequest {
            kind: BackdropKind::Horizon,
            shown: 1.0,
        },
    );
    if ui.input(|input| input.focused) && super::motion::ambient_allowed(ui.ctx()) {
        ui.ctx().request_repaint_after(backdrop::FRAME);
    }
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(24)))
        .show(ui, |ui| {
            let space = ui.max_rect();
            let column = egui::Rect::from_center_size(
                space.center(),
                egui::vec2(space.width().min(CONTENT_WIDTH), space.height()),
            );
            let window = ui.ctx().content_rect();
            let with_wall = window.width() >= WALL_MIN_WINDOW_WIDTH;
            if with_wall {
                wall::draw(ui, wall_area(window, column));
            }
            ui.scope_builder(egui::UiBuilder::new().max_rect(column), |ui| {
                theme::split_row(ui, theme::CONTROL_HEIGHT, theme::brand, |ui| {
                    if theme::icon_button(ui, icons::SETTINGS, "Settings", true).clicked() {
                        action = Some(StartAction::OpenSettings);
                    }
                    theme::fullscreen_button(ui);
                });
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let width = ui.available_width().min(FORM_WIDTH);
                    let inset = if with_wall {
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

/// From the form's gap to the window's own edges, past the panel's margin, so the wall bleeds off
/// the top, right and bottom rather than floating in a box.
fn wall_area(window: egui::Rect, column: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(column.left() + FORM_WIDTH + COLUMN_GAP, window.top()),
        window.max,
    )
}

fn form(
    ui: &mut egui::Ui,
    state: &mut StartState,
    recent: &[RecentRoom],
    now_unix: u64,
    update_state: &UpdateState,
    action: &mut Option<StartAction>,
) {
    if state.invited {
        invite_form(ui, state, action);
        return;
    }
    ui.label(egui::RichText::new("Share your screen").font(theme::semibold(28.0)));
    ui.label(
        egui::RichText::new("Peer to peer, no account needed")
            .size(14.0)
            .color(theme::secondary()),
    );
    ui.add_space(24.0);
    theme::section_heading(ui, "Your name");
    ui.add_enabled_ui(!state.connecting, |ui| {
        theme::field(
            ui,
            egui::TextEdit::singleline(&mut state.nickname).hint_text("Your name"),
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
            egui::TextEdit::singleline(&mut state.ticket)
                .hint_text("Paste an invite link or ticket"),
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
    open_status(ui, state);
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

/// What a link opens: one question, the name the room would see, and a way out. Worded as a question
/// rather than "You're invited" because a stranger's web page can send someone here too.
fn invite_form(ui: &mut egui::Ui, state: &mut StartState, action: &mut Option<StartAction>) {
    ui.label(egui::RichText::new("Join this room?").font(theme::semibold(28.0)));
    ui.label(
        egui::RichText::new(
            "A link sent you here. People in the room can see your name and IP address.",
        )
        .size(14.0)
        .color(theme::secondary()),
    );
    ui.add_space(24.0);
    theme::section_heading(ui, "Your name");
    let name = ui
        .add_enabled_ui(!state.connecting, |ui| {
            theme::field(
                ui,
                egui::TextEdit::singleline(&mut state.nickname).hint_text("Your name"),
                theme::FORM_HEIGHT,
            )
        })
        .inner;
    let full_width = egui::vec2(ui.available_width(), theme::FORM_HEIGHT);
    let join = theme::wide_action(
        ui,
        icons::JOIN,
        "Join room",
        theme::Tone::Primary,
        !state.connecting,
        full_width,
    );
    if std::mem::take(&mut state.focus_join) {
        join.request_focus();
    }
    if join.clicked() {
        *action = Some(StartAction::Join);
    }
    ui.add_space(4.0);
    let not_now = theme::text_button(ui, "Not now", !state.connecting);
    // Esc in the name field only leaves the field, and an open dialog takes it for itself.
    let dialog_open = ui.ctx().memory(|memory| memory.top_modal_layer()).is_some();
    let editing = name.has_focus() || name.lost_focus();
    let escape = !dialog_open
        && !editing
        && !state.connecting
        && ui.input(|input| input.key_pressed(egui::Key::Escape));
    if not_now.clicked() || escape {
        *action = Some(StartAction::Decline);
    }
    open_status(ui, state);
}

/// How the open in flight is going, or why the last one failed.
fn open_status(ui: &mut egui::Ui, state: &StartState) {
    if state.connecting {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            theme::spinner(ui, 16.0);
            ui.weak("Connecting to the room…");
        });
    }
    if !state.error.is_empty() {
        ui.add_space(8.0);
        ui.colored_label(theme::error(), &state.error);
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

    fn sized(width: f32, height: f32) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, height),
            )),
            ..Default::default()
        }
    }

    fn run(
        ctx: &egui::Context,
        state: &mut StartState,
        raw: egui::RawInput,
    ) -> (egui::FullOutput, Option<StartAction>) {
        let mut action = None;
        let mut output = ctx.run_ui(raw, |ui| {
            action = draw(ui, state, &[], 0, &UpdateState::new(false));
        });
        output.textures_delta.clear();
        (output, action)
    }

    #[test]
    fn the_wall_reaches_the_windows_edges_and_stops_short_of_the_form() {
        // The column is the window less 24-point margins, at most 1200 wide and centred.
        for (width, height, column_left, wall_left) in [
            (960.0, 540.0, 24.0, 468.0),
            (1300.0, 780.0, 50.0, 494.0),
            (2560.0, 1440.0, 680.0, 1124.0),
            (3840.0, 2160.0, 1320.0, 1764.0),
        ] {
            let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
            let column = egui::Rect::from_min_size(
                egui::pos2(column_left, 24.0),
                egui::vec2(1200.0, height - 48.0),
            );
            assert_eq!(
                wall_area(window, column),
                egui::Rect::from_min_max(egui::pos2(wall_left, 0.0), egui::pos2(width, height))
            );
        }
    }

    #[test]
    fn the_wall_paints_past_the_panels_margin_to_the_windows_edges() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        wall::install_loaded(&ctx);
        let mut state = StartState::new("Alex".into());
        let (output, _) = run(&ctx, &mut state, sized(1300.0, 780.0));
        let clips: Vec<egui::Rect> = output
            .shapes
            .iter()
            .filter(|clipped| matches!(clipped.shape, egui::Shape::Mesh(_)))
            .map(|clipped| clipped.clip_rect)
            .collect();
        assert!(!clips.is_empty(), "no tiles were painted");
        let expected = egui::Rect::from_min_max(egui::pos2(494.0, 0.0), egui::pos2(1300.0, 780.0));
        assert!(clips.iter().all(|clip| *clip == expected), "{clips:?}");
    }

    #[test]
    fn below_960_points_the_start_screen_paints_no_wall() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut state = StartState::new("Alex".into());
        let (output, _) = run(&ctx, &mut state, sized(959.0, 700.0));
        assert!(
            !output
                .shapes
                .iter()
                .any(|clipped| matches!(clipped.shape, egui::Shape::Mesh(_)))
        );
        assert!(!wall::has_state(&ctx));
        run(&ctx, &mut state, sized(960.0, 700.0));
        assert!(wall::has_state(&ctx));
    }

    #[test]
    fn a_tile_under_settings_is_not_hovered_and_settings_takes_the_click() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        wall::install_loaded(&ctx);
        let mut state = StartState::new("Alex".into());
        let with_events = |events: Vec<egui::Event>| {
            let mut raw = sized(1440.0, 900.0);
            raw.events = events;
            raw
        };
        // The 1200-point column ends at 1320, and the 36-point button sits at its top right.
        let on_settings = egui::pos2(1302.0, 42.0);
        let on_the_wall = egui::pos2(1000.0, 450.0);
        for _ in 0..3 {
            run(
                &ctx,
                &mut state,
                with_events(vec![egui::Event::PointerMoved(on_the_wall)]),
            );
        }
        assert!(
            wall::hovered(&ctx).is_some(),
            "the wall takes a free pointer"
        );
        for _ in 0..3 {
            run(
                &ctx,
                &mut state,
                with_events(vec![egui::Event::PointerMoved(on_settings)]),
            );
        }
        assert_eq!(wall::hovered(&ctx), None);
        let button = |pressed| egui::Event::PointerButton {
            pos: on_settings,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        run(&ctx, &mut state, with_events(vec![button(true)]));
        let (_, action) = run(&ctx, &mut state, with_events(vec![button(false)]));
        assert_eq!(action, Some(StartAction::OpenSettings));
    }

    fn press(key: egui::Key) -> egui::RawInput {
        let mut raw = sized(1440.0, 900.0);
        raw.events = vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }];
        raw
    }

    fn drawn_text(output: &egui::FullOutput) -> String {
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    fn invited(ticket: &RoomTicket) -> StartState {
        let mut state = StartState::new("Alex".into());
        state.invite(ticket.to_string());
        state
    }

    #[test]
    fn an_invite_asks_first_and_shows_only_what_answers_it() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut state = invited(&valid_ticket());
        let (output, action) = run(&ctx, &mut state, sized(1440.0, 900.0));
        let text = drawn_text(&output);
        assert_eq!(action, None, "a link alone must not join");
        for shown in ["Join this room?", "IP address", "Your name", "Not now"] {
            assert!(text.contains(shown), "{shown} missing from {text}");
        }
        for hidden in ["Create room", "Have an invite?"] {
            assert!(!text.contains(hidden), "{hidden} shown in {text}");
        }
    }

    #[test]
    fn enter_answers_an_invite_with_join() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let ticket = valid_ticket();
        let mut state = invited(&ticket);
        run(&ctx, &mut state, sized(1440.0, 900.0));
        let (_, action) = run(&ctx, &mut state, press(egui::Key::Enter));
        assert_eq!(action, Some(StartAction::Join));
        assert_eq!(
            state.submit(StartAction::Join),
            Some(Intent::Join(ticket)),
            "joining from an invite opens the room the link named"
        );
    }

    #[test]
    fn escape_declines_an_invite_and_the_form_returns_without_the_ticket() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut state = invited(&valid_ticket());
        run(&ctx, &mut state, sized(1440.0, 900.0));
        let (_, action) = run(&ctx, &mut state, press(egui::Key::Escape));
        assert_eq!(action, Some(StartAction::Decline));
        assert_eq!(state.submit(StartAction::Decline), None);
        assert!(!state.invited);
        assert!(state.ticket.is_empty());
        let (output, _) = run(&ctx, &mut state, sized(1440.0, 900.0));
        assert!(drawn_text(&output).contains("Create room"));
    }

    #[test]
    fn a_failed_join_stays_on_the_invite_with_its_reason() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut state = invited(&valid_ticket());
        state.submit(StartAction::Join);
        state.failed("no room member answered".into());
        let (output, _) = run(&ctx, &mut state, sized(1440.0, 900.0));
        let text = drawn_text(&output);
        assert!(text.contains("Join this room?"), "{text}");
        assert!(text.contains("no room member answered"), "{text}");
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
        assert!(
            state.error.starts_with("That link or ticket is not valid"),
            "{}",
            state.error
        );
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
