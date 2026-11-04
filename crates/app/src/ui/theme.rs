//! OpenStream's deep-water control-room material, shared by every native window.
use std::sync::Arc;

use egui::{
    Color32, FontData, FontDefinitions, FontFamily, FontId, FontTweak, Margin, Response, RichText,
    Sense, Stroke, TextStyle, Vec2, WidgetInfo, WidgetText, WidgetType,
};

use super::motion;

/// Video letterboxes on black, so the canvas between tiles stays black and tiles read as screens.
pub const CANVAS: Color32 = Color32::BLACK;
pub const GROUND: Color32 = Color32::from_rgb(10, 16, 23);
pub const SURFACE: Color32 = Color32::from_rgb(17, 26, 35);
pub const RAISED: Color32 = Color32::from_rgb(27, 38, 50);
pub const HIGHLIGHT: Color32 = Color32::from_rgb(39, 53, 68);
pub const PRESSED: Color32 = Color32::from_rgb(52, 70, 89);
pub const RULE: Color32 = Color32::from_rgb(31, 43, 55);
pub const TEXT: Color32 = Color32::from_rgb(238, 245, 250);
pub const SECONDARY: Color32 = Color32::from_rgb(163, 179, 194);
pub const MUTED: Color32 = Color32::from_rgb(125, 143, 160);
pub const ACCENT: Color32 = Color32::from_rgb(92, 200, 255);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(132, 215, 255);
pub const ACCENT_PRESSED: Color32 = Color32::from_rgb(62, 172, 235);
/// Ink on an accent surface.
pub const ON_ACCENT: Color32 = Color32::from_rgb(4, 22, 36);
pub const SELECTED: Color32 = Color32::from_rgb(22, 66, 97);
pub const SELECTED_HOVER: Color32 = Color32::from_rgb(29, 84, 122);
/// A live signal is the accent, so the one hue on screen always means the same thing.
pub const LIVE: Color32 = ACCENT;
pub const ERROR: Color32 = Color32::from_rgb(255, 142, 147);
pub const WARNING: Color32 = Color32::from_rgb(239, 192, 110);

/// Below this window width the room rail folds into the dock's sheets.
pub const COMPACT_WIDTH: f32 = 800.0;
pub const CONTROL_HEIGHT: f32 = 36.0;
/// The start screen's fields and buttons, where a few large controls are all there is.
pub const FORM_HEIGHT: f32 = 40.0;
pub const RADIUS_CHIP: u8 = 6;
pub const RADIUS_CONTROL: u8 = 8;
pub const RADIUS_CARD: u8 = 12;
pub const RADIUS_DIALOG: u8 = 16;
const ICON_SIZE: f32 = 16.0;
const MEDIUM_FAMILY: &str = "medium";
const SEMIBOLD_FAMILY: &str = "semibold";
const ICON_FAMILY: &str = "icons";

/// Lucide codepoints present in `assets/fonts/Lucide-subset.ttf`.
pub mod icons {
    pub const SHARE_MONITOR: &str = "\u{e422}";
    pub const SHARE_WINDOW: &str = "\u{e426}";
    pub const STOP: &str = "\u{e486}";
    pub const COPIED: &str = "\u{e06c}";
    pub const LINK: &str = "\u{e102}";
    pub const TICKET: &str = "\u{e20f}";
    pub const SETTINGS: &str = "\u{e154}";
    pub const VOLUME: &str = "\u{e1ab}";
    pub const MUTED: &str = "\u{e1ac}";
    pub const FULLSCREEN: &str = "\u{e112}";
    pub const WINDOWED: &str = "\u{e11a}";
    pub const POP_OUT: &str = "\u{e3af}";
    pub const CLOSE: &str = "\u{e1b2}";
    pub const STATS: &str = "\u{e068}";
    pub const UPLOAD: &str = "\u{e19e}";
    pub const ROOM: &str = "\u{e1a4}";
    pub const BACK_TO_GRID: &str = "\u{e457}";
    pub const CREATE: &str = "\u{e13d}";
    pub const JOIN: &str = "\u{e10d}";
    pub const REFRESH: &str = "\u{e145}";
    pub const APPLICATIONS: &str = "\u{e55a}";
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_global_style(style());
}

/// IBM Plex Sans leads each family; egui's bundled faces stay behind it as the fallback for scripts
/// it lacks. Its digits all share one width, so a figure that changes while it is watched holds still
/// without a second face.
fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    // Measured on rendered controls rather than derived from the metrics, which leave out the
    // rounding of a row onto whole pixels: this puts the middle of a label's capitals on the middle
    // of its button, and on the middle of an icon beside it.
    let optical_centre = FontTweak {
        y_offset_factor: -0.02,
        ..Default::default()
    };
    let faces: [(&str, &'static [u8], FontFamily); 3] = [
        (
            "ibm-plex-sans-regular",
            include_bytes!("../../assets/fonts/IBMPlexSans-Regular.ttf"),
            FontFamily::Proportional,
        ),
        (
            "ibm-plex-sans-medium",
            include_bytes!("../../assets/fonts/IBMPlexSans-Medium.ttf"),
            FontFamily::Name(MEDIUM_FAMILY.into()),
        ),
        (
            "ibm-plex-sans-semibold",
            include_bytes!("../../assets/fonts/IBMPlexSans-SemiBold.ttf"),
            FontFamily::Name(SEMIBOLD_FAMILY.into()),
        ),
    ];
    for (name, bytes, family) in faces {
        fonts.font_data.insert(
            name.to_owned(),
            Arc::new(FontData::from_static(bytes).tweak(optical_centre.clone())),
        );
        let chain = std::iter::once(name.to_owned())
            .chain(fallback.iter().cloned())
            .collect();
        fonts.families.insert(family, chain);
    }
    fonts.font_data.insert(
        ICON_FAMILY.to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Lucide-subset.ttf"
        ))),
    );
    fonts.families.insert(
        FontFamily::Name(ICON_FAMILY.into()),
        vec![ICON_FAMILY.to_owned()],
    );
    fonts
}

fn style() -> egui::Style {
    let mut style = egui::Style {
        text_styles: [
            (TextStyle::Heading, semibold(16.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, medium(14.0)),
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into(),
        animation_time: 0.16,
        ..Default::default()
    };
    style.interaction.selectable_labels = false;
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 8.0);
    style.spacing.interact_size = Vec2::new(36.0, 24.0);
    style.spacing.icon_spacing = 8.0;
    style.spacing.window_margin = Margin::same(24);
    style.spacing.menu_margin = Margin::same(6);
    style.spacing.slider_width = 88.0;
    style.spacing.combo_width = 104.0;
    style.spacing.indent = 16.0;
    // Floats over the content rather than taking a column from it, so a rail that starts to scroll
    // keeps its cards on the same right edge as the buttons above them.
    style.spacing.scroll = egui::style::ScrollStyle::floating();
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = GROUND;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = GROUND;
    visuals.text_edit_bg_color = Some(GROUND);
    visuals.faint_bg_color = SURFACE;
    visuals.code_bg_color = RAISED;
    visuals.weak_text_color = Some(SECONDARY);
    visuals.window_highlight_topmost = false;
    visuals.window_corner_radius = RADIUS_DIALOG.into();
    visuals.menu_corner_radius = RADIUS_CARD.into();
    visuals.window_stroke = Stroke::new(1.0, RULE);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = ERROR;
    visuals.hyperlink_color = ACCENT;
    visuals.slider_trailing_fill = true;
    visuals.selection.bg_fill = SELECTED;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.text_cursor.stroke = Stroke::new(1.5, ACCENT);
    visuals.handle_shape = egui::style::HandleShape::Circle;
    for (widget, fill, border, ink, expansion) in [
        (
            &mut visuals.widgets.noninteractive,
            SURFACE,
            RULE,
            TEXT,
            0.0,
        ),
        (&mut visuals.widgets.inactive, RAISED, RAISED, TEXT, 0.0),
        (
            &mut visuals.widgets.hovered,
            HIGHLIGHT,
            HIGHLIGHT,
            TEXT,
            0.0,
        ),
        // egui reuses `active` for keyboard focus, so the bright border doubles as the focus ring.
        (&mut visuals.widgets.active, PRESSED, ACCENT, TEXT, 0.0),
        (&mut visuals.widgets.open, HIGHLIGHT, HIGHLIGHT, TEXT, 0.0),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0, border);
        widget.fg_stroke = Stroke::new(1.5, ink);
        widget.corner_radius = RADIUS_CONTROL.into();
        widget.expansion = expansion;
    }
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    // `bg_fill` is what checkboxes, radios and slider rails use; they sit on surfaces a button's
    // fill would melt into.
    visuals.widgets.inactive.bg_fill = HIGHLIGHT;
    style.visuals = visuals;
    style
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM_FAMILY.into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD_FAMILY.into()))
}

pub fn icon(glyph: &str) -> RichText {
    RichText::new(glyph).font(FontId::new(ICON_SIZE, FontFamily::Name(ICON_FAMILY.into())))
}

/// A figure that changes while it is watched. Plex's digits share one width, so it does not jitter.
pub fn figure(text: impl Into<String>) -> RichText {
    RichText::new(text).small().color(SECONDARY)
}

/// A room ticket, in the one place the interface uses a monospace: characters are the data here,
/// and two tickets are told apart by comparing them column by column.
pub fn ticket_text(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace().color(SECONDARY)
}

pub fn is_compact(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() < COMPACT_WIDTH
}

pub fn rail() -> egui::Frame {
    egui::Frame::NONE.fill(GROUND).inner_margin(16)
}

pub fn card() -> egui::Frame {
    egui::Frame::NONE
        .fill(SURFACE)
        .corner_radius(RADIUS_CARD)
        .inner_margin(12)
}

pub fn section_heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).font(medium(13.0)).color(SECONDARY));
}

/// One row of controls, `left` from the start and `right` from the end. Sizing the row up front
/// keeps a right-to-left group from claiming the whole height of a panel that fits its content.
pub fn split_row(
    ui: &mut egui::Ui,
    height: f32,
    left: impl FnOnce(&mut egui::Ui),
    right: impl FnOnce(&mut egui::Ui),
) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            left(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
        },
    );
}

/// Gives every control in this `ui` one height, so a row mixing drag values, dropdowns, buttons
/// and switches lines up.
pub fn uniform_controls(ui: &mut egui::Ui) {
    let spacing = ui.spacing_mut();
    spacing.interact_size.y = CONTROL_HEIGHT;
    spacing.button_padding.y = 0.0;
}

/// A setting: its label on one fixed column and its control on another, so a stack of these reads
/// as two clean edges. Where the columns would crush the control the label goes above it instead.
pub fn form_row<R>(ui: &mut egui::Ui, label: &str, control: impl FnOnce(&mut egui::Ui) -> R) -> R {
    const LABEL_COLUMN: f32 = 152.0;
    const STACK_BELOW: f32 = 400.0;
    let body = |ui: &mut egui::Ui| {
        uniform_controls(ui);
        ui.spacing_mut().item_spacing.y = 4.0;
        control(ui)
    };
    if ui.available_width() < STACK_BELOW {
        section_heading(ui, label);
        return body(ui);
    }
    ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(LABEL_COLUMN - ui.spacing().item_spacing.x, CONTROL_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_size(ui.available_size());
                ui.label(RichText::new(label).color(SECONDARY));
            },
        );
        ui.vertical(body).inner
    })
    .inner
}

/// Centres what `add` draws on the height left in `ui`. egui places things before it knows how tall
/// they are, so this uses the height measured last pass and asks for another pass when that changes,
/// which keeps every frame that reaches the screen correct.
pub fn vertically_centred<R>(
    ui: &mut egui::Ui,
    id_salt: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let id = ui.id().with(("vertically-centred", id_salt));
    let measured = ui
        .ctx()
        .data(|data| data.get_temp::<f32>(id))
        .unwrap_or(0.0);
    ui.add_space(((ui.available_height() - measured) / 2.0).max(0.0));
    let scope = ui.scope(add);
    let height = scope.response.rect.height();
    if (height - measured).abs() > 0.5 {
        ui.ctx().data_mut(|data| data.insert_temp(id, height));
        ui.ctx().request_discard("centred content changed height");
    }
    scope.inner
}

/// A chevron in place of egui's filled triangle, in the ink of the control it sits on. It points
/// down shut and up open, turning over on a spring. `id` belongs to its dropdown, because the `ui`
/// egui hands an icon has the same id for every sibling.
fn chevron(
    ui: &egui::Ui,
    id: egui::Id,
    rect: egui::Rect,
    visuals: &egui::style::WidgetVisuals,
    open: bool,
) {
    let turned = motion::follow(ui.ctx(), id, open, motion::HOVER);
    paint_chevron(
        ui.painter(),
        rect.center(),
        rect.width().min(8.0) / 2.0,
        (1.0 - 2.0 * turned) * 2.5,
        visuals.fg_stroke.color,
    );
}

/// `rise` is how far the tip sits below the arms, negative for pointing up.
fn paint_chevron(
    painter: &egui::Painter,
    centre: egui::Pos2,
    half_width: f32,
    rise: f32,
    ink: Color32,
) {
    painter.add(egui::Shape::line(
        vec![
            centre + egui::vec2(-half_width, -rise),
            centre + egui::vec2(0.0, rise),
            centre + egui::vec2(half_width, -rise),
        ],
        Stroke::new(1.5, ink),
    ));
}

/// What a control's surface is at rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    /// The one action on a screen that matters most, and its only accent surface.
    Primary,
    /// No surface until the pointer arrives, for rows in a list.
    Quiet,
}

/// Wraps any egui control in one physical behaviour. Its own surface is switched off and drawn
/// here instead, under it, from springs: hover eases in and out, a press lands on pointer-down and
/// lets go slowly, and selection and focus ease too. All of them can be reversed mid-flight.
pub fn tactile(
    ui: &mut egui::Ui,
    tone: Tone,
    selected: bool,
    enabled: bool,
    add: impl FnOnce(&mut egui::Ui) -> Response,
) -> Response {
    const PRESS_INSET: f32 = 1.0;
    let backing = ui.painter().add(egui::Shape::Noop);
    // Read before the scope takes the next id: a control's tone can change while it stays put.
    let accent = motion::follow(
        ui.ctx(),
        ui.next_auto_id().with("accent"),
        tone == Tone::Primary,
        motion::TRAVEL,
    );
    let response = ui
        .scope(|ui| {
            if !enabled {
                ui.disable();
            }
            let visuals = ui.visuals_mut();
            visuals.selection.bg_fill = Color32::TRANSPARENT;
            let widgets = &mut visuals.widgets;
            for widget in [
                &mut widgets.noninteractive,
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
                &mut widgets.open,
            ] {
                widget.bg_fill = Color32::TRANSPARENT;
                widget.weak_bg_fill = Color32::TRANSPARENT;
                widget.bg_stroke = Stroke::NONE;
                widget.expansion = 0.0;
                if enabled {
                    widget.fg_stroke.color =
                        widget.fg_stroke.color.lerp_to_gamma(ON_ACCENT, accent);
                }
            }
            add(ui)
        })
        .inner;
    let (ctx, id) = (ui.ctx().clone(), response.id);
    let hover = motion::follow(&ctx, id.with("hover"), response.hovered(), motion::HOVER);
    let held = motion::press(&ctx, id.with("held"), response.is_pointer_button_down_on());
    let chosen = motion::follow(&ctx, id.with("chosen"), selected, motion::HOVER);
    let live = motion::follow(&ctx, id.with("enabled"), enabled, motion::TRAVEL);
    let focus = motion::follow(&ctx, id.with("focus"), response.has_focus(), motion::HOVER);
    let (rest, over, down) = match tone {
        Tone::Primary | Tone::Neutral => (
            RAISED.lerp_to_gamma(ACCENT, accent),
            HIGHLIGHT.lerp_to_gamma(ACCENT_HOVER, accent),
            PRESSED.lerp_to_gamma(ACCENT_PRESSED, accent),
        ),
        Tone::Quiet => (Color32::TRANSPARENT, HIGHLIGHT, PRESSED),
    };
    let fill = SURFACE
        .lerp_to_gamma(rest, live)
        .lerp_to_gamma(SELECTED, chosen)
        .lerp_to_gamma(over.lerp_to_gamma(SELECTED_HOVER, chosen), hover)
        .lerp_to_gamma(down, held);
    let surface = response.rect.shrink(held * PRESS_INSET);
    let mut shapes = vec![egui::Shape::rect_filled(surface, RADIUS_CONTROL, fill)];
    if focus > 0.0 {
        shapes.push(egui::Shape::rect_stroke(
            response.rect,
            RADIUS_CONTROL,
            Stroke::new(1.0, ACCENT.gamma_multiply(focus)),
            egui::StrokeKind::Outside,
        ));
    }
    ui.painter().set(backing, egui::Shape::Vec(shapes));
    response
}

/// A text field in the same material as every other control. Its own frame is switched off and
/// drawn here, so its focus ring eases in the way a button's does.
pub fn field(ui: &mut egui::Ui, edit: egui::TextEdit<'_>, height: f32) -> Response {
    let backing = ui.painter().add(egui::Shape::Noop);
    // A single-line field takes its height from one row of text plus its margin alone.
    let margin_y = ((height - ui.text_style_height(&TextStyle::Body)) / 2.0).round() as i8;
    let response = ui.add(
        edit.frame(egui::Frame::new().inner_margin(Margin::symmetric(12, margin_y)))
            .vertical_align(egui::Align::Center)
            .desired_width(f32::INFINITY),
    );
    let (ctx, id) = (ui.ctx().clone(), response.id);
    let hover = motion::follow(&ctx, id.with("hover"), response.hovered(), motion::HOVER);
    let focus = motion::follow(&ctx, id.with("focus"), response.has_focus(), motion::HOVER);
    let border = HIGHLIGHT
        .lerp_to_gamma(PRESSED, hover)
        .lerp_to_gamma(ACCENT, focus);
    ui.painter().set(
        backing,
        egui::epaint::RectShape::new(
            response.rect,
            RADIUS_CONTROL,
            GROUND,
            Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        ),
    );
    response
}

/// Centre the visible ink, since the icon and text fonts have different ascenders.
fn labelled_button(ui: &mut egui::Ui, glyph: &str, text: &str, size: Vec2) -> Response {
    let ink = ui.visuals().widgets.inactive.fg_stroke.color;
    let icon = ui.painter().layout_no_wrap(
        glyph.to_owned(),
        FontId::new(ICON_SIZE, FontFamily::Name(ICON_FAMILY.into())),
        ink,
    );
    let label = ui
        .painter()
        .layout_no_wrap(text.to_owned(), medium(14.0), ink);
    let gap = if text.is_empty() || glyph.is_empty() {
        0.0
    } else {
        8.0
    };
    // Descenders (the p in Copy, for example) must not lift the whole label above its icon.
    let capitals = ui.painter().layout_no_wrap("H".into(), medium(14.0), ink);
    let icon_bounds = icon.mesh_bounds;
    let label_bounds = label.mesh_bounds;
    let icon_width = if glyph.is_empty() {
        0.0
    } else {
        icon_bounds.width()
    };
    let label_width = if text.is_empty() {
        0.0
    } else {
        label_bounds.width()
    };
    let width = icon_width + gap + label_width;
    let desired = egui::vec2(width + 2.0 * ui.spacing().button_padding.x, CONTROL_HEIGHT).max(size);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let left = rect.center().x - width / 2.0;
    for (galley, x, centre_y) in [
        (icon, left - icon_bounds.left(), icon_bounds.center().y),
        (
            label,
            left + icon_width + gap - label_bounds.left(),
            capitals.mesh_bounds.center().y,
        ),
    ] {
        if !galley.mesh_bounds.is_finite() {
            continue;
        }
        ui.painter()
            .galley(egui::pos2(x, rect.center().y - centre_y), galley, ink);
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), text));
    response
}

pub fn action(ui: &mut egui::Ui, glyph: &str, text: &str, enabled: bool) -> Response {
    tactile(ui, Tone::Neutral, false, enabled, |ui| {
        labelled_button(ui, glyph, text, Vec2::ZERO)
    })
}

pub fn primary_action(ui: &mut egui::Ui, glyph: &str, text: &str, enabled: bool) -> Response {
    tactile(ui, Tone::Primary, false, enabled, |ui| {
        labelled_button(ui, glyph, text, Vec2::ZERO)
    })
}

/// A button of a set size with its content centred, for buttons that line up with what is around
/// them.
pub fn wide_action(
    ui: &mut egui::Ui,
    glyph: &str,
    text: &str,
    tone: Tone,
    enabled: bool,
    size: Vec2,
) -> Response {
    tactile(ui, tone, false, enabled, |ui| {
        labelled_button(ui, glyph, text, size)
    })
}

pub fn text_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> Response {
    tactile(ui, Tone::Neutral, false, enabled, |ui| {
        labelled_button(ui, "", text, Vec2::ZERO)
    })
}

pub fn primary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> Response {
    tactile(ui, Tone::Primary, false, enabled, |ui| {
        labelled_button(ui, "", text, Vec2::ZERO)
    })
}

/// A label's padding would make an icon button wider than it is tall.
fn square_padding(ui: &mut egui::Ui) {
    ui.spacing_mut().button_padding.x = (CONTROL_HEIGHT - ICON_SIZE) / 2.0;
}

/// An icon with no visible label still has to name itself, to the pointer and to a screen reader.
pub fn icon_button(ui: &mut egui::Ui, glyph: &str, name: &str, enabled: bool) -> Response {
    toned_icon_button(ui, Tone::Neutral, glyph, name, enabled)
}

pub fn toned_icon_button(
    ui: &mut egui::Ui,
    tone: Tone,
    glyph: &str,
    name: &str,
    enabled: bool,
) -> Response {
    let response = tactile(ui, tone, false, enabled, |ui| {
        square_padding(ui);
        labelled_button(ui, glyph, "", Vec2::splat(CONTROL_HEIGHT))
    });
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, name));
    response.on_hover_text(name)
}

/// A control that stays pressed. Flips `on` and reports the change.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool, glyph: &str, text: &str) -> Response {
    let selected = *on;
    let mut response = tactile(ui, Tone::Neutral, selected, true, |ui| {
        labelled_button(ui, glyph, text, Vec2::ZERO)
    });
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), selected, text));
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response
}

pub fn icon_toggle(ui: &mut egui::Ui, on: &mut bool, glyph: &str, name: &str) -> Response {
    let selected = *on;
    let mut response = tactile(ui, Tone::Neutral, selected, true, |ui| {
        square_padding(ui);
        labelled_button(ui, glyph, "", Vec2::splat(CONTROL_HEIGHT))
    });
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, *on, name));
    response.on_hover_text(name)
}

/// A dropdown with the same surface and height as every other control.
pub fn dropdown(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    text: impl Into<WidgetText>,
    width: Option<f32>,
    items: impl FnOnce(&mut egui::Ui),
) -> Response {
    let chevron_id = egui::Id::new(("dropdown-chevron", &id_salt));
    tactile(ui, Tone::Neutral, false, true, |ui| {
        uniform_controls(ui);
        let mut combo = egui::ComboBox::from_id_salt(id_salt)
            .icon(move |ui, rect, visuals, open| chevron(ui, chevron_id, rect, visuals, open))
            .selected_text(text);
        if let Some(width) = width {
            combo = combo.width(width);
        }
        combo.show_ui(ui, items).response
    })
}

/// A drag value with the same surface and height as every other control.
pub fn drag_value(ui: &mut egui::Ui, value: egui::DragValue<'_>) -> Response {
    tactile(ui, Tone::Neutral, false, true, |ui| {
        uniform_controls(ui);
        ui.add(value)
    })
}

/// Exclusive choices sharing one track, with the selection as a thumb that travels between them,
/// retargeting from wherever it has got to. Returns the choice clicked.
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id_salt: &str,
    options: &[(T, &str)],
    selected: T,
    segment_width: f32,
) -> Option<T> {
    const THUMB_INSET: f32 = 3.0;
    let size = egui::vec2(segment_width * options.len() as f32, CONTROL_HEIGHT);
    let (track, _) = ui.allocate_exact_size(size, Sense::hover());
    let id = ui.id().with(("segmented", id_salt));
    let ctx = ui.ctx().clone();
    let index = options.iter().position(|(value, _)| *value == selected);
    let at = motion::glide(
        &ctx,
        id.with("thumb"),
        index.unwrap_or_default() as f32,
        motion::TRAVEL,
    );
    let segment = |at: f32| {
        egui::Rect::from_min_size(
            track.min + egui::vec2(at * segment_width, 0.0),
            egui::vec2(segment_width, track.height()),
        )
    };
    ui.painter().rect_filled(track, RADIUS_CONTROL, RAISED);
    if index.is_some() {
        ui.painter().rect_filled(
            segment(at).shrink(THUMB_INSET),
            RADIUS_CONTROL - THUMB_INSET as u8,
            SELECTED,
        );
    }
    let mut clicked = None;
    for (position, (value, text)) in options.iter().enumerate() {
        let rect = segment(position as f32);
        let response = ui.interact(rect, id.with(position), Sense::click());
        let chosen = index == Some(position);
        response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, chosen, *text));
        let lit = motion::follow(
            &ctx,
            response.id.with("lit"),
            chosen || response.hovered(),
            motion::HOVER,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            *text,
            medium(14.0),
            SECONDARY.lerp_to_gamma(TEXT, lit),
        );
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                RADIUS_CONTROL - 1,
                Stroke::new(1.0, ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        if response.clicked() {
            clicked = Some(*value);
        }
    }
    clicked
}

/// Allocates a drawn indicator and its label as one clickable row, returning where to draw the
/// indicator. A label too long for the row is cut with an ellipsis and offered whole in a tooltip.
fn indicator_row(ui: &mut egui::Ui, indicator: Vec2, text: &str) -> (Response, egui::Rect) {
    const GAP: f32 = 8.0;
    // The row's whole width rather than what is left of it: in a wrapped row an item that does not
    // fit moves to the next line, and must not be cut short before it gets there.
    let room = (ui.max_rect().width() - indicator.x - GAP).max(0.0);
    let galley = WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        room,
        TextStyle::Body,
    );
    let elided = galley.elided;
    let size = egui::vec2(
        indicator.x + GAP + galley.size().x,
        ui.spacing().interact_size.y.max(indicator.y),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let glyph = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.center().y - indicator.y / 2.0),
        indicator,
    );
    if ui.is_rect_visible(rect) {
        let ink = if ui.is_enabled() { TEXT } else { MUTED };
        ui.painter().galley(
            egui::pos2(
                glyph.right() + GAP,
                rect.center().y
                    - ui.painter()
                        .layout_no_wrap("H".into(), FontId::proportional(14.0), ink)
                        .mesh_bounds
                        .center()
                        .y,
            ),
            galley,
            ink,
        );
    }
    let response = if elided {
        response.on_hover_text(text)
    } else {
        response
    };
    (response, glyph)
}

/// The fill an off/on indicator takes, shared so a radio, a check and a switch read as one family.
fn indicator_fill(hover: f32, on: f32) -> Color32 {
    HIGHLIGHT
        .lerp_to_gamma(PRESSED, hover)
        .lerp_to_gamma(ACCENT.lerp_to_gamma(ACCENT_HOVER, hover), on)
}

/// One choice of several, whose dot grows out of the middle when chosen. Sets `current` and reports
/// the change.
pub fn radio<T: PartialEq>(ui: &mut egui::Ui, current: &mut T, value: T, text: &str) -> Response {
    const SIZE: f32 = 16.0;
    const DOT_RADIUS: f32 = 3.0;
    let (mut response, glyph) = indicator_row(ui, Vec2::splat(SIZE), text);
    let mut selected = *current == value;
    if response.clicked() && !selected {
        *current = value;
        response.mark_changed();
        selected = true;
    }
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, enabled, selected, text));
    let ctx = ui.ctx().clone();
    let on = motion::follow(&ctx, response.id, selected, motion::TRAVEL);
    let hover = motion::follow(
        &ctx,
        response.id.with("hover"),
        response.hovered(),
        motion::HOVER,
    );
    let fade = if enabled { 1.0 } else { 0.5 };
    let painter = ui.painter();
    painter.circle_filled(
        glyph.center(),
        SIZE / 2.0,
        indicator_fill(hover, on).gamma_multiply(fade),
    );
    if on > 0.0 {
        painter.circle_filled(glyph.center(), DOT_RADIUS * on, ON_ACCENT);
    }
    if response.has_focus() {
        painter.circle_stroke(glyph.center(), SIZE / 2.0 + 2.0, Stroke::new(1.0, ACCENT));
    }
    response
}

/// One option that can be on alongside others. Flips `on` and reports the change.
pub fn check(ui: &mut egui::Ui, on: &mut bool, text: &str) -> Response {
    const SIZE: f32 = 16.0;
    const RADIUS: f32 = 4.0;
    let (mut response, glyph) = indicator_row(ui, Vec2::splat(SIZE), text);
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, *on, text));
    let ctx = ui.ctx().clone();
    let shown = motion::follow(&ctx, response.id, *on, motion::TRAVEL);
    let hover = motion::follow(
        &ctx,
        response.id.with("hover"),
        response.hovered(),
        motion::HOVER,
    );
    let fade = if enabled { 1.0 } else { 0.5 };
    let painter = ui.painter();
    painter.rect_filled(
        glyph,
        RADIUS,
        indicator_fill(hover, shown).gamma_multiply(fade),
    );
    if shown > 0.0 {
        let centre = glyph.center();
        painter.add(egui::Shape::line(
            vec![
                centre + egui::vec2(-3.5, 0.0),
                centre + egui::vec2(-1.0, 2.5),
                centre + egui::vec2(3.5, -2.5),
            ],
            Stroke::new(1.5, ON_ACCENT.gamma_multiply(shown)),
        ));
    }
    if response.has_focus() {
        painter.rect_stroke(
            glyph.expand(2.0),
            RADIUS + 2.0,
            Stroke::new(1.0, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    response
}

/// A section that folds: its name on the left edge and a chevron on the right that turns over as it
/// opens. The caller holds whether it is open and draws the content with [`motion::unfold`].
pub fn disclosure(ui: &mut egui::Ui, text: &str, open: bool) -> Response {
    const HEIGHT: f32 = 28.0;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), HEIGHT), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::CollapsingHeader, true, open, text));
    let ctx = ui.ctx().clone();
    let turned = motion::follow(&ctx, response.id, open, motion::TRAVEL);
    let hover = motion::follow(
        &ctx,
        response.id.with("hover"),
        response.hovered(),
        motion::HOVER,
    );
    let ink = SECONDARY.lerp_to_gamma(TEXT, hover);
    let painter = ui.painter();
    painter.text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        medium(13.0),
        ink,
    );
    paint_chevron(
        painter,
        rect.right_center() - egui::vec2(8.0, 0.0),
        4.0,
        (1.0 - 2.0 * turned) * 2.5,
        ink,
    );
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            RADIUS_CHIP,
            Stroke::new(1.0, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    response
}

/// An on/off switch whose thumb travels when flipped and stretches while held. The flip retargets
/// from wherever the thumb is, so a second click mid-travel reverses it without a jump.
pub fn switch(ui: &mut egui::Ui, on: &mut bool, text: &str) -> Response {
    const TRACK: Vec2 = Vec2::new(30.0, 18.0);
    const THUMB_RADIUS: f32 = 6.0;
    const THUMB_INSET: f32 = 3.0;
    const HELD_STRETCH: f32 = 3.0;
    let (mut response, track) = indicator_row(ui, TRACK, text);
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, *on, text));
    if ui.is_rect_visible(response.rect) {
        let ctx = ui.ctx().clone();
        let t = motion::follow(&ctx, response.id, *on, motion::TRAVEL);
        let hover = motion::follow(
            &ctx,
            response.id.with("hover"),
            response.hovered(),
            motion::HOVER,
        );
        let held = motion::press(
            &ctx,
            response.id.with("held"),
            response.is_pointer_button_down_on(),
        );
        let fill = indicator_fill(hover, t);
        let thumb_fill = SECONDARY.lerp_to_gamma(ON_ACCENT, t);
        let painter = ui.painter();
        painter.rect_filled(track, TRACK.y / 2.0, fill);
        if response.has_focus() {
            painter.rect_stroke(
                track.expand(2.0),
                TRACK.y / 2.0 + 2.0,
                Stroke::new(1.0, ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        let stretch = held * HELD_STRETCH;
        let left = egui::lerp(
            track.left() + THUMB_INSET..=track.right() - THUMB_INSET - 2.0 * THUMB_RADIUS - stretch,
            t,
        );
        let thumb = egui::Rect::from_min_size(
            egui::pos2(left, track.center().y - THUMB_RADIUS),
            egui::vec2(2.0 * THUMB_RADIUS + stretch, 2.0 * THUMB_RADIUS),
        );
        painter.rect_filled(thumb, THUMB_RADIUS, thumb_fill);
    }
    response
}

/// A spinner that asks for 20 frames a second rather than every frame. `egui::Spinner` always
/// requests an immediate repaint, and a stream can sit in Reconnecting for as long as its publisher
/// stays away, which would keep the window drawing flat out over nothing.
pub fn spinner(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    spinner_at(ui, rect, 1.0);
}

pub fn spinner_at(ui: &egui::Ui, rect: egui::Rect, opacity: f32) {
    const FRAME: std::time::Duration = std::time::Duration::from_millis(50);
    const SWEEP: f32 = 1.5 * std::f32::consts::PI;
    const SEGMENTS: usize = 24;
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.ctx().request_repaint_after(FRAME);
    let start = ui.input(|i| i.time) as f32 * std::f32::consts::TAU;
    let radius = rect.width().min(rect.height()) / 2.0 - 1.0;
    let points = (0..=SEGMENTS)
        .map(|step| {
            let angle = start + SWEEP * step as f32 / SEGMENTS as f32;
            rect.center() + radius * Vec2::angled(angle)
        })
        .collect();
    ui.painter().add(egui::Shape::line(
        points,
        Stroke::new(2.0, ACCENT.gamma_multiply(opacity)),
    ));
}

pub fn live_dot(ui: &mut egui::Ui, live: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 16.0), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 3.5, if live { LIVE } else { MUTED });
}

pub fn live_status(ui: &mut egui::Ui, text: &str, live: bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        live_dot(ui, live);
        ui.label(RichText::new(text).color(if live { LIVE } else { SECONDARY }));
    });
}

pub fn brand(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        crate::icon::paint_mark(ui, 26.0);
        ui.label(RichText::new("OpenStream").font(semibold(18.0)));
    });
}

/// A modal dialog: dims the room behind it and rises a few points as it arrives, and leaves the
/// way it came when `visible` goes false, carrying on from wherever it had got to. Returns nothing
/// once it has fully gone. Otherwise returns what `content` produced and whether the user dismissed
/// it with Esc or the close button. A click on the backdrop does not count, because these dialogs
/// hold drafts a stray click must not discard.
///
/// While it leaves it takes no input and blocks nothing, so the room responds at once.
pub fn dialog<T>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    visible: bool,
    content: impl FnOnce(&mut egui::Ui) -> T,
) -> Option<(T, bool)> {
    const RISE: f32 = 12.0;
    const WIDTH: f32 = 480.0;
    const WINDOW_MARGIN: f32 = 16.0;
    const PADDING: f32 = 24.0;
    let id = egui::Id::new(id);
    let shown = motion::appear(ctx, id.with("presence"), visible, motion::SURFACE);
    if !visible && shown <= 0.0 {
        return None;
    }
    // Sampled before the pass: a popup reacts to Esc while it is drawn and closes itself, so
    // reading this afterwards would always see it as already closed.
    let popup_open = egui::Popup::is_any_open(ctx);
    let width = WIDTH.min(ctx.content_rect().width() - 2.0 * WINDOW_MARGIN);
    let inner_width = (width - 2.0 * PADDING).max(0.0);
    let frame = egui::Frame::popup(&ctx.global_style())
        .corner_radius(RADIUS_DIALOG)
        .inner_margin(PADDING as i8)
        .multiply_with_opacity(shown);
    let scrim = Color32::from_black_alpha(170).gamma_multiply(shown);
    let area = egui::Modal::default_area(id)
        .fade_in(false)
        .anchor(egui::Align2::CENTER_CENTER, motion::rise(shown, RISE));
    let mut closed = false;
    let body = |ui: &mut egui::Ui| {
        ui.set_opacity(shown);
        ui.set_width(inner_width);
        split_row(
            ui,
            CONTROL_HEIGHT,
            |ui| {
                ui.label(RichText::new(title).font(semibold(18.0)));
            },
            |ui| {
                closed = toned_icon_button(ui, Tone::Quiet, icons::CLOSE, "Close", true).clicked();
            },
        );
        ui.add_space(12.0);
        content(ui)
    };
    if !visible {
        let inner = area
            .interactable(false)
            .show(ctx, |ui| {
                ui.painter().rect_filled(ctx.content_rect(), 0.0, scrim);
                frame.show(ui, body).inner
            })
            .inner;
        return Some((inner, false));
    }
    let response = egui::Modal::new(id)
        .frame(frame)
        .backdrop_color(scrim)
        .area(area)
        .show(ctx, body);
    // A source list can arrive while Settings is open; one Esc closes only the dialog on top.
    let escape =
        response.is_top_modal && !popup_open && ctx.input(|i| i.key_pressed(egui::Key::Escape));
    Some((response.inner, closed || escape))
}

/// The row that ends a dialog: its actions end on the content's right edge with the main one last,
/// and anything secondary sits on the left. `actions` is laid out from the right, so it adds the main
/// action first.
pub fn dialog_footer(
    ui: &mut egui::Ui,
    secondary: impl FnOnce(&mut egui::Ui),
    actions: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(12.0);
    split_row(ui, CONTROL_HEIGHT, secondary, actions);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_and_label_ink_are_centred_in_controls() {
        let ctx = egui::Context::default();
        install(&ctx);
        for width in [0.0, 380.0] {
            let mut rects = Vec::new();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                for (glyph, text) in [(icons::CREATE, "Create room"), (icons::LINK, "Copy link")] {
                    rects.push(
                        wide_action(
                            ui,
                            glyph,
                            text,
                            Tone::Neutral,
                            true,
                            egui::vec2(width, FORM_HEIGHT),
                        )
                        .rect,
                    );
                }
            });
            output.textures_delta.clear();
            let ink: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some(text.galley.mesh_bounds.translate(text.pos.to_vec2()))
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(ink.len(), rects.len() * 2);
            for (rect, pair) in rects.iter().zip(ink.as_chunks::<2>().0) {
                for bounds in pair {
                    assert!((bounds.center().y - rect.center().y).abs() <= 2.0);
                    assert!(rect.contains_rect(*bounds));
                }
                assert!((pair[0].union(pair[1]).center().x - rect.center().x).abs() <= 2.0);
            }
        }
    }

    #[test]
    fn descenders_do_not_shift_labels_relative_to_icons() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.horizontal(|ui| {
                for text in ["Create room", "Copy link", "Copy ticket", "Join room"] {
                    action(ui, icons::CREATE, text, true);
                }
            });
        });
        output.textures_delta.clear();
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text().len() > 3
                {
                    return Some(text.pos.y);
                }
                None
            })
            .collect();
        assert_eq!(labels.len(), 4);
        assert!(labels.iter().all(|y| (*y - labels[0]).abs() < 0.01));
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let luminance = |c: Color32| {
            let linear = |v: u8| {
                let v = f32::from(v) / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn every_text_role_stays_readable_on_every_surface_it_sits_on() {
        for ink in [TEXT, SECONDARY, ACCENT, WARNING, ERROR] {
            for surface in [GROUND, SURFACE, RAISED] {
                assert!(contrast(ink, surface) >= 4.5, "{ink:?} on {surface:?}");
            }
        }
        for surface in [GROUND, SURFACE] {
            assert!(contrast(MUTED, surface) >= 4.5, "muted on {surface:?}");
        }
        for surface in [ACCENT, ACCENT_HOVER, ACCENT_PRESSED] {
            assert!(contrast(ON_ACCENT, surface) >= 4.5, "ink on {surface:?}");
        }
        for ink in [TEXT, ACCENT] {
            for surface in [SELECTED, SELECTED_HOVER] {
                assert!(contrast(ink, surface) >= 3.0, "{ink:?} on {surface:?}");
            }
        }
    }

    #[test]
    fn the_interface_font_is_ibm_plex_sans_with_the_bundled_faces_as_fallback() {
        let fonts = fonts();
        let proportional = &fonts.families[&FontFamily::Proportional];
        assert_eq!(proportional[0], "ibm-plex-sans-regular");
        assert!(proportional.len() > 1, "scripts it lacks need a fallback");
    }

    #[test]
    fn every_digit_is_one_width_so_live_figures_cannot_jitter() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for font in [FontId::proportional(12.0), medium(14.0), semibold(16.0)] {
                let widths: Vec<f32> = ('0'..='9')
                    .map(|digit| ui.fonts_mut(|fonts| fonts.glyph_width(&font, digit)))
                    .collect();
                assert!(
                    widths.iter().all(|width| *width == widths[0]),
                    "{font:?} digits differ: {widths:?}"
                );
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn every_icon_is_in_the_subset() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for glyph in [
                icons::SHARE_MONITOR,
                icons::SHARE_WINDOW,
                icons::STOP,
                icons::COPIED,
                icons::LINK,
                icons::TICKET,
                icons::SETTINGS,
                icons::VOLUME,
                icons::MUTED,
                icons::FULLSCREEN,
                icons::WINDOWED,
                icons::POP_OUT,
                icons::CLOSE,
                icons::STATS,
                icons::UPLOAD,
                icons::ROOM,
                icons::BACK_TO_GRID,
                icons::CREATE,
                icons::JOIN,
                icons::REFRESH,
                icons::APPLICATIONS,
            ] {
                let font = FontId::new(ICON_SIZE, FontFamily::Name(ICON_FAMILY.into()));
                let has_glyph = ui.fonts_mut(|fonts| fonts.has_glyphs(&font, glyph));
                assert!(has_glyph, "{glyph:?} is not in Lucide-subset.ttf");
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn operating_system_light_mode_cannot_override_the_dark_interface() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut output = ctx.run_ui(
            egui::RawInput {
                system_theme: Some(egui::Theme::Light),
                ..Default::default()
            },
            |ui| {
                assert!(ui.visuals().dark_mode);
                assert_eq!(ui.visuals().panel_fill, GROUND);
            },
        );
        output.textures_delta.clear();
    }
}
