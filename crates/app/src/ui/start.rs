//! The start screen: nickname and ticket entry with Create and Join, shown until a room is open.

use super::theme::{self, icons};
use super::update::{self, UpdateState};
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

/// Wide enough to keep the form on the brand's edge with the field beside it, rather than centred.
const FORM_BESIDE_FIELD_MIN_WIDTH: f32 = 960.0;
const FORM_WIDTH: f32 = 380.0;
/// The brand and the form share this column, so on a very wide window they stay on one common
/// edge instead of drifting to the corner of the screen. The window's own buttons do sit at its.
const CONTENT_WIDTH: f32 = 1200.0;
/// Room above the logo and the window's buttons, past the panel's margin.
const HEADER_TOP_GAP: f32 = 12.0;

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
    if ui.input(|input| input.focused) && super::motion::ambient_allowed(ui.ctx()) {
        ui.ctx().request_repaint_after(backdrop::FRAME);
    }
    // The brand, the form and the window's buttons, measured as they are laid out.
    let mut quiet = [egui::Rect::NOTHING; 3];
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(24)))
        .show(ui, |ui| {
            let space = ui.max_rect();
            let column = egui::Rect::from_center_size(
                space.center(),
                egui::vec2(space.width().min(CONTENT_WIDTH), space.height()),
            );
            let form_on_the_left = ui.ctx().content_rect().width() >= FORM_BESIDE_FIELD_MIN_WIDTH;
            ui.scope_builder(egui::UiBuilder::new().max_rect(space), |ui| {
                ui.add_space(HEADER_TOP_GAP);
                theme::split_row(
                    ui,
                    theme::CONTROL_HEIGHT,
                    |_| {},
                    |ui| {
                        if theme::icon_button(ui, icons::SETTINGS, "Settings", true).clicked() {
                            action = Some(StartAction::OpenSettings);
                        }
                        theme::fullscreen_button(ui);
                        quiet[2] = ui.min_rect();
                    },
                );
            });
            ui.scope_builder(egui::UiBuilder::new().max_rect(column), |ui| {
                ui.add_space(HEADER_TOP_GAP);
                theme::split_row(
                    ui,
                    theme::CONTROL_HEIGHT,
                    |ui| {
                        theme::brand(ui);
                        quiet[0] = ui.min_rect();
                    },
                    |_| {},
                );
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let width = ui.available_width().min(FORM_WIDTH);
                    let inset = if form_on_the_left {
                        0.0
                    } else {
                        ((ui.available_width() - width) / 2.0).max(0.0)
                    };
                    theme::vertically_centred(ui, "start-form", |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(inset);
                            quiet[1] = ui
                                .vertical(|ui| {
                                    ui.set_width(width);
                                    form(ui, state, recent, now_unix, update_state, &mut action);
                                })
                                .response
                                .rect;
                        });
                    });
                });
            });
        });
    backdrop::request(
        ui.ctx(),
        BackdropRequest {
            kind: BackdropKind::Field { quiet },
            shown: 1.0,
            signal: 1.0,
        },
    );
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
    use std::time::Duration;

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

    /// The window at 1440 by 900, `n` frames of a 60-a-second clock in, so springs settle.
    fn frame(n: u32, focused: bool) -> egui::RawInput {
        egui::RawInput {
            time: Some(f64::from(n) / 60.0),
            focused,
            ..sized(1440.0, 900.0)
        }
    }

    #[test]
    fn settings_takes_a_click_on_the_start_screen() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut state = StartState::new("Alex".into());
        let with_events = |events: Vec<egui::Event>| {
            let mut raw = sized(1440.0, 900.0);
            raw.events = events;
            raw
        };
        // The panel's margin ends at 1416, and the 32-point button sits at its top right, 12 down.
        let on_settings = egui::pos2(1400.0, 52.0);
        for _ in 0..3 {
            run(
                &ctx,
                &mut state,
                with_events(vec![egui::Event::PointerMoved(on_settings)]),
            );
        }
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

    #[test]
    fn the_start_screen_asks_for_frames_only_while_the_field_can_move() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        crate::ui::motion::set_less_motion(&ctx, false);
        let mut state = StartState::new("Alex".into());
        let delay = |output: &egui::FullOutput| {
            output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
        };
        let focused = (0..=120)
            .map(|n| run(&ctx, &mut state, frame(n, true)).0)
            .last()
            .unwrap();
        assert!(
            delay(&focused) > Duration::ZERO && delay(&focused) <= backdrop::FRAME,
            "{:?}",
            delay(&focused)
        );
        let unfocused = (121..=240)
            .map(|n| run(&ctx, &mut state, frame(n, false)).0)
            .last()
            .unwrap();
        assert_eq!(delay(&unfocused), Duration::MAX);
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

    fn text_rect(output: &egui::FullOutput, needle: &str) -> egui::Rect {
        output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text().contains(needle) => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no text containing {needle:?}"))
    }

    #[test]
    fn the_field_stays_quiet_around_the_brand_the_form_and_the_window_buttons() {
        for (width, height) in [
            (640.0, 480.0),
            (960.0, 540.0),
            (1280.0, 800.0),
            (1920.0, 1080.0),
            (3840.0, 2160.0),
        ] {
            let ctx = egui::Context::default();
            theme::install(&ctx);
            let mut state = StartState::new("Alex".into());
            state.error = "Couldn't reach the room".into();
            // The form is centred from the height measured on the previous pass.
            let output = (0..3)
                .map(|_| run(&ctx, &mut state, sized(width, height)).0)
                .last()
                .unwrap();
            let Some(BackdropRequest {
                kind:
                    BackdropKind::Field {
                        quiet: [brand, form, buttons],
                    },
                ..
            }) = backdrop::take_request(&ctx)
            else {
                panic!("the start screen asked for no field at {width}");
            };
            assert!(
                brand
                    .expand(1.0)
                    .contains_rect(text_rect(&output, "OpenStream")),
                "{width}"
            );
            for line in [
                "Share your screen",
                "Your name",
                "Create room",
                "Have an invite?",
                "Join room",
                "Couldn't reach the room",
            ] {
                assert!(
                    form.expand(1.0).contains_rect(text_rect(&output, line)),
                    "{line} at {width}"
                );
            }
            assert!(
                buttons.max.x >= width - 25.0 && buttons.min.y <= 37.0 && buttons.width() >= 64.0,
                "{buttons:?}"
            );
            if width >= 960.0 {
                assert!(
                    brand.max.x <= width / 2.0 && form.max.x <= width / 2.0,
                    "{form:?} at {width}"
                );
            } else {
                assert!(
                    (form.center().x - width / 2.0).abs() <= 1.0,
                    "{form:?} at {width}"
                );
            }
        }
    }

    #[test]
    fn an_invite_keeps_the_field_quiet_around_its_question() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut state = invited(&valid_ticket());
        let output = (0..3)
            .map(|_| run(&ctx, &mut state, sized(1440.0, 900.0)).0)
            .last()
            .unwrap();
        let Some(BackdropRequest {
            kind: BackdropKind::Field {
                quiet: [_, form, _],
            },
            ..
        }) = backdrop::take_request(&ctx)
        else {
            panic!("no field");
        };
        assert!(form.expand(1.0).contains_rect(text_rect(&output, "Join")));
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
