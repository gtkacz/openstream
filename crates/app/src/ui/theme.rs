//! OpenStream's Terminal material, shared by every native window. Colours are read through functions
//! so the phosphor can change while the app runs.
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use egui::emath::GuiRounding as _;
use egui::{
    Color32, FontData, FontDefinitions, FontFamily, FontId, FontTweak, Margin, Response, RichText,
    Sense, Stroke, TextStyle, Vec2, WidgetInfo, WidgetText, WidgetType,
};
use serde::{Deserialize, Serialize};

use super::motion;

/// Video letterboxes on black, so the canvas between tiles stays black and tiles read as screens.
pub const CANVAS: Color32 = Color32::BLACK;

/// The colour of the tube.
// shortcut: only `Blue` is reachable while choosing a colour is disabled. To bring it back, restore
// the Settings row and the `start_with_phosphor` call at launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phosphor {
    #[default]
    Blue = 0,
    Amber = 1,
    Green = 2,
    White = 3,
}

static PHOSPHOR: AtomicU8 = AtomicU8::new(0);

pub fn phosphor() -> Phosphor {
    match PHOSPHOR.load(Ordering::Relaxed) {
        1 => Phosphor::Amber,
        2 => Phosphor::Green,
        3 => Phosphor::White,
        _ => Phosphor::Blue,
    }
}

/// Before any window exists, so the first frame is already the right colour.
pub fn start_with_phosphor(phosphor: Phosphor) {
    PHOSPHOR.store(phosphor as u8, Ordering::Relaxed);
}

/// Also rebuilds this window's style, which bakes a few colours in.
pub fn set_phosphor(ctx: &egui::Context, phosphor: Phosphor) {
    start_with_phosphor(phosphor);
    ctx.set_global_style(style());
    ctx.request_repaint();
}

/// How a window other than the one Settings is open in learns about a new phosphor. Most colours are
/// read fresh every frame, but a window's style bakes a few in.
pub fn keep_up_with_phosphor(ctx: &egui::Context) {
    let id = egui::Id::new("styled-for-phosphor");
    let current = phosphor();
    if ctx.data(|data| data.get_temp::<Phosphor>(id)) != Some(current) {
        ctx.data_mut(|data| data.insert_temp(id, current));
        ctx.set_global_style(style());
    }
}

struct Palette {
    ground: Color32,
    surface: Color32,
    raised: Color32,
    selected: Color32,
    rule: Color32,
    wire: Color32,
    text: Color32,
    secondary: Color32,
    muted: Color32,
    accent: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

const BLUE: Palette = Palette {
    ground: hex(0x0E1022),
    surface: hex(0x131429),
    raised: hex(0x141C4A),
    selected: hex(0x17236B),
    rule: hex(0x192982),
    wire: hex(0x1E3FD8),
    text: hex(0xF6F8FF),
    secondary: hex(0xA5D8FF),
    muted: hex(0x6C94E1),
    accent: hex(0x38BDF8),
};
const AMBER: Palette = Palette {
    ground: hex(0x120B03),
    surface: hex(0x1A1105),
    raised: hex(0x2A1C08),
    selected: hex(0x3D2A0B),
    rule: hex(0x4A330F),
    wire: hex(0x8A6016),
    text: hex(0xFFC552),
    secondary: hex(0xD9971A),
    muted: hex(0xA97416),
    accent: hex(0xFFB000),
};
const GREEN: Palette = Palette {
    ground: hex(0x030F08),
    surface: hex(0x06170D),
    raised: hex(0x0A2414),
    selected: hex(0x0E3A1F),
    rule: hex(0x0F4424),
    wire: hex(0x17803F),
    text: hex(0x8BFFB0),
    secondary: hex(0x27C95E),
    muted: hex(0x1E9F4C),
    accent: hex(0x33FF77),
};
const WHITE: Palette = Palette {
    ground: hex(0x0A0D0F),
    surface: hex(0x11161A),
    raised: hex(0x1C2328),
    selected: hex(0x2A3339),
    rule: hex(0x2F393E),
    wire: hex(0x66747B),
    text: hex(0xF2F7F8),
    secondary: hex(0xAEBBC1),
    muted: hex(0x86949B),
    accent: hex(0xDCE6EA),
};

fn palette() -> &'static Palette {
    match phosphor() {
        Phosphor::Blue => &BLUE,
        Phosphor::Amber => &AMBER,
        Phosphor::Green => &GREEN,
        Phosphor::White => &WHITE,
    }
}

pub fn ground() -> Color32 {
    palette().ground
}
pub fn surface() -> Color32 {
    palette().surface
}
pub fn raised() -> Color32 {
    palette().raised
}
pub fn highlight() -> Color32 {
    palette().selected
}
pub fn pressed() -> Color32 {
    palette().rule
}
pub fn rule() -> Color32 {
    palette().rule
}
/// The circuit in the rail and on the room map.
pub fn wire() -> Color32 {
    palette().wire
}
pub fn text() -> Color32 {
    palette().text
}
pub fn secondary() -> Color32 {
    palette().secondary
}
pub fn muted() -> Color32 {
    palette().muted
}
pub fn accent() -> Color32 {
    palette().accent
}
/// The one main action on a screen.
pub fn main_action() -> Color32 {
    palette().wire
}
pub fn accent_pressed() -> Color32 {
    palette().secondary
}
/// Ink on an accent surface: reverse video.
pub fn on_accent() -> Color32 {
    palette().ground
}
pub fn selected() -> Color32 {
    palette().selected
}
pub fn selected_hover() -> Color32 {
    palette().raised
}
pub fn live() -> Color32 {
    palette().accent
}
/// The same in every phosphor, because an alarm must not change with the tube.
pub fn error() -> Color32 {
    hex(0xFF2D78)
}
pub fn warning() -> Color32 {
    palette().text
}

/// Below this window width the room rail folds into the dock's sheets.
pub const COMPACT_WIDTH: f32 = 800.0;
pub const CONTROL_HEIGHT: f32 = 32.0;
/// The start screen's fields and buttons, where a few large controls are all there is.
pub const FORM_HEIGHT: f32 = 40.0;
pub const RADIUS_CHIP: u8 = 0;
pub const RADIUS_CONTROL: u8 = 0;
pub const RADIUS_CARD: u8 = 0;
pub const RADIUS_DIALOG: u8 = 0;
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

/// IBM Plex Mono leads each family, so everything sits on one character grid; egui's bundled faces
/// stay behind it as the fallback for scripts it lacks.
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
        scale: 40.0 / 42.0,
        y_offset_factor: -0.02,
        ..Default::default()
    };
    let faces: [(&str, &'static [u8], FontFamily); 3] = [
        (
            "ibm-plex-mono-regular",
            include_bytes!("../../assets/fonts/IBMPlexMono-Regular.ttf"),
            FontFamily::Proportional,
        ),
        (
            "ibm-plex-mono-medium",
            include_bytes!("../../assets/fonts/IBMPlexMono-Medium.ttf"),
            FontFamily::Name(MEDIUM_FAMILY.into()),
        ),
        (
            "ibm-plex-mono-semibold",
            include_bytes!("../../assets/fonts/IBMPlexMono-SemiBold.ttf"),
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
    if let Some(chain) = fonts.families.get(&FontFamily::Proportional).cloned() {
        fonts.families.insert(FontFamily::Monospace, chain);
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
            (TextStyle::Heading, semibold(14.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, medium(14.0)),
            (TextStyle::Small, FontId::proportional(14.0)),
            (TextStyle::Monospace, FontId::monospace(14.0)),
        ]
        .into(),
        animation_time: 0.16,
        // What text laid out directly gets, so a widget's label lands on the same pixel row as a
        // painted one. egui's default centres each glyph in its line, a pixel lower at 1×, and one
        // font to a row leaves it nothing to align.
        override_text_valign: Some(egui::Align::BOTTOM),
        ..Default::default()
    };
    style.interaction.selectable_labels = false;
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(8.0, 8.0);
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
    visuals.panel_fill = ground();
    visuals.window_fill = surface();
    visuals.extreme_bg_color = ground();
    visuals.text_edit_bg_color = Some(ground());
    visuals.faint_bg_color = surface();
    visuals.code_bg_color = raised();
    visuals.weak_text_color = Some(secondary());
    visuals.window_highlight_topmost = false;
    visuals.window_corner_radius = RADIUS_DIALOG.into();
    visuals.menu_corner_radius = RADIUS_CARD.into();
    visuals.window_stroke = Stroke::new(1.0, wire());
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    visuals.popup_shadow = egui::epaint::Shadow::NONE;
    visuals.warn_fg_color = warning();
    visuals.error_fg_color = error();
    visuals.hyperlink_color = accent();
    visuals.slider_trailing_fill = true;
    visuals.selection.bg_fill = selected();
    visuals.selection.stroke = Stroke::new(1.0, accent());
    visuals.text_cursor.stroke = Stroke::new(2.0, accent());
    visuals.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.5 };
    for (widget, fill, border, ink, expansion) in [
        (
            &mut visuals.widgets.noninteractive,
            surface(),
            rule(),
            text(),
            0.0,
        ),
        (&mut visuals.widgets.inactive, raised(), rule(), text(), 0.0),
        (
            &mut visuals.widgets.hovered,
            accent(),
            accent(),
            on_accent(),
            0.0,
        ),
        // egui reuses `active` for keyboard focus, so the bright border doubles as the focus ring.
        (
            &mut visuals.widgets.active,
            text(),
            accent(),
            on_accent(),
            0.0,
        ),
        (&mut visuals.widgets.open, selected(), wire(), text(), 0.0),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0, border);
        widget.fg_stroke = Stroke::new(1.5, ink);
        widget.corner_radius = RADIUS_CONTROL.into();
        widget.expansion = expansion;
    }
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text());
    // `bg_fill` is what checkboxes, radios and slider rails use; they sit on surfaces a button's
    // fill would melt into.
    visuals.widgets.inactive.bg_fill = highlight();
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

/// A figure that changes while it is watched.
pub fn figure(text: impl Into<String>) -> RichText {
    RichText::new(text).color(secondary())
}

/// A room ticket: characters are the data here, and two tickets are told apart column by column.
pub fn ticket_text(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace().color(secondary())
}

pub fn is_compact(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() < COMPACT_WIDTH
}

pub fn rail() -> egui::Frame {
    egui::Frame::NONE.fill(ground()).inner_margin(16)
}

pub fn card() -> egui::Frame {
    egui::Frame::NONE
        .stroke(Stroke::new(1.0, rule()))
        .inner_margin(8)
}

/// Static scanlines and grain under a surface. It is one tile that never moves, so it adds tooth to
/// the chrome without ever asking for a frame, and it sits under the text so text keeps its edges.
pub fn paint_material(ui: &egui::Ui, rect: egui::Rect) {
    const SIZE: usize = 192;
    let id = egui::Id::new("material-tile");
    let ctx = ui.ctx();
    let texture = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id));
    let texture = texture.unwrap_or_else(|| {
        let mut seed = 0x2545_F491u32;
        let pixels = (0..SIZE * SIZE)
            .map(|at| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let scan = if (at / SIZE).is_multiple_of(3) { 13 } else { 0 };
                Color32::from_white_alpha(scan + (seed % 10) as u8)
            })
            .collect();
        let image = egui::ColorImage::new([SIZE, SIZE], pixels);
        let options = egui::TextureOptions {
            wrap_mode: egui::TextureWrapMode::Repeat,
            ..egui::TextureOptions::NEAREST
        };
        let handle = ctx.load_texture("material", image, options);
        ctx.data_mut(|data| data.insert_temp(id, handle.clone()));
        handle
    });
    let scale = ctx.pixels_per_point() / SIZE as f32;
    let uv = egui::Rect::from_min_max(
        (rect.min.to_vec2() * scale).to_pos2(),
        (rect.max.to_vec2() * scale).to_pos2(),
    );
    let mut mesh = egui::Mesh::with_texture(texture.id());
    mesh.add_rect_with_uv(rect, uv, wire());
    ui.painter().add(egui::Shape::mesh(mesh));
}

/// Someone's mark, drawn from their public key. Mirrored like a face, because people recognise a
/// symmetrical shape far faster than a noisy one. It helps people recognise each other and proves
/// nothing, the way a nickname proves nothing.
pub fn key_glyph(painter: &egui::Painter, centre: egui::Pos2, key: &[u8], cell: f32, ink: Color32) {
    const GRID: i32 = 7;
    let origin = centre - Vec2::splat(cell * GRID as f32 / 2.0);
    let bit = |index: usize| {
        key.get(index / 8)
            .is_some_and(|byte| byte >> (index % 8) & 1 == 1)
    };
    let mut index = 0;
    for y in 0..GRID {
        for x in 0..4 {
            if bit(index) {
                for column in [x, GRID - 1 - x] {
                    let min = origin + egui::vec2(column as f32 * cell, y as f32 * cell);
                    let rect = egui::Rect::from_min_size(min, Vec2::splat(cell))
                        .round_to_pixels(painter.pixels_per_point());
                    painter.rect_filled(rect, 0, ink);
                }
            }
            index += 1;
        }
    }
}

pub fn section_heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).font(medium(13.0)).color(secondary()));
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
                ui.label(RichText::new(label).color(secondary()));
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
    let rest = Color32::TRANSPARENT.lerp_to_gamma(main_action(), accent);
    let fill = Color32::TRANSPARENT
        .lerp_to_gamma(rest, live)
        .lerp_to_gamma(self::selected(), chosen)
        .lerp_to_gamma(self::accent(), hover)
        .lerp_to_gamma(text(), held);
    let surface = response.rect.shrink(held * PRESS_INSET);
    let mut shapes = vec![egui::Shape::rect_filled(surface, RADIUS_CONTROL, fill)];
    if focus > 0.0 {
        shapes.push(egui::Shape::rect_stroke(
            response.rect,
            RADIUS_CONTROL,
            Stroke::new(1.0, self::accent().gamma_multiply(focus)),
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
    // The caret is painted below as a block. Not a scope for this: one could not wrap in a row.
    let usual_caret = std::mem::replace(&mut ui.visuals_mut().text_cursor.stroke, Stroke::NONE);
    let output = edit
        .frame(egui::Frame::new().inner_margin(Margin::symmetric(12, margin_y)))
        .vertical_align(egui::Align::Center)
        .desired_width(f32::INFINITY)
        .show(ui);
    ui.visuals_mut().text_cursor.stroke = usual_caret;
    let response = output.response.response;
    if response.has_focus()
        && let Some(range) = output.cursor_range
    {
        block_caret(
            ui,
            &output.galley,
            output.galley_pos,
            // Not the text's own clip, which ends where the text does: that is where a caret usually is.
            response.rect,
            range.primary,
        );
    }
    let (ctx, id) = (ui.ctx().clone(), response.id);
    let hover = motion::follow(&ctx, id.with("hover"), response.hovered(), motion::HOVER);
    let focus = motion::follow(&ctx, id.with("focus"), response.has_focus(), motion::HOVER);
    let border = rule()
        .lerp_to_gamma(wire(), hover)
        .lerp_to_gamma(accent(), focus);
    ui.painter().set(
        backing,
        egui::epaint::RectShape::new(
            response.rect,
            RADIUS_CONTROL,
            Color32::BLACK,
            Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        ),
    );
    response
}

/// A solid cell over the character the next keystroke lands on, with that character in reverse video.
/// It holds still for someone who asked their system for less motion.
fn block_caret(
    ui: &egui::Ui,
    galley: &egui::Galley,
    at: egui::Pos2,
    clip: egui::Rect,
    cursor: egui::text::CCursor,
) {
    const BLINK_SECONDS: f64 = 0.6;
    let less = motion::asked_for_less(ui.ctx());
    let now = ui.input(|input| input.time);
    let lit = less || now.rem_euclid(BLINK_SECONDS * 2.0) < BLINK_SECONDS;
    if !less {
        let wait = BLINK_SECONDS - now.rem_euclid(BLINK_SECONDS);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(wait));
    }
    if !lit {
        return;
    }
    let font = TextStyle::Body.resolve(ui.style());
    let width = ui
        .painter()
        .layout_no_wrap("0".to_owned(), font.clone(), accent())
        .size()
        .x;
    let place = galley.pos_from_cursor(cursor).translate(at.to_vec2());
    let cell = egui::Rect::from_min_size(place.min, egui::vec2(width, place.height()));
    let painter = ui.painter().with_clip_rect(clip.intersect(ui.clip_rect()));
    painter.rect_filled(cell, 0, accent());
    if let Some(under) = galley.text().chars().nth(cursor.index.0) {
        painter.text(
            cell.center(),
            egui::Align2::CENTER_CENTER,
            under,
            font,
            on_accent(),
        );
    }
}

/// A bracketed label, or a solid block for the main action, or an icon alone where a label will
/// not fit. The ink flips with the hover and press the surface under it is showing.
fn labelled_button(ui: &mut egui::Ui, glyph: &str, text: &str, size: Vec2, tone: Tone) -> Response {
    let base = ui.visuals().widgets.inactive.fg_stroke.color;
    let shown = match (text.is_empty(), tone) {
        (true, _) => glyph.to_owned(),
        (false, Tone::Primary | Tone::Quiet) => text.to_owned(),
        (false, _) => format!("[ {text} ]"),
    };
    let font = if text.is_empty() {
        FontId::new(ICON_SIZE, FontFamily::Name(ICON_FAMILY.into()))
    } else {
        medium(14.0)
    };
    // Laid out without a colour of its own: one set here would ignore the ink it is painted with.
    let label = ui
        .painter()
        .layout_no_wrap(shown, font, Color32::PLACEHOLDER);
    let capitals = ui.painter().layout_no_wrap("H".into(), medium(14.0), base);
    let width = label.mesh_bounds.width();
    let desired = egui::vec2(width + 2.0 * ui.spacing().button_padding.x, CONTROL_HEIGHT).max(size);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let ctx = ui.ctx().clone();
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
    let ink = base.lerp_to_gamma(on_accent(), hover.max(held));
    let centre_y = if text.is_empty() {
        label.mesh_bounds.center().y
    } else {
        capitals.mesh_bounds.center().y
    };
    if label.mesh_bounds.is_finite() {
        let at = egui::pos2(
            rect.center().x - width / 2.0 - label.mesh_bounds.left(),
            rect.center().y - centre_y,
        );
        ui.painter().galley(at, label, ink);
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), text));
    response
}

pub fn action(ui: &mut egui::Ui, glyph: &str, text: &str, enabled: bool) -> Response {
    let tone_here = Tone::Neutral;
    tactile(ui, tone_here, false, enabled, |ui| {
        labelled_button(ui, glyph, text, Vec2::ZERO, tone_here)
    })
}

pub fn primary_action(ui: &mut egui::Ui, glyph: &str, text: &str, enabled: bool) -> Response {
    let tone_here = Tone::Primary;
    tactile(ui, tone_here, false, enabled, |ui| {
        labelled_button(ui, glyph, text, Vec2::ZERO, tone_here)
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
        labelled_button(ui, glyph, text, size, tone)
    })
}

/// A label with no brackets, for a bar that already frames its controls.
pub fn quiet_action(ui: &mut egui::Ui, text: &str) -> Response {
    tactile(ui, Tone::Quiet, false, true, |ui| {
        labelled_button(ui, "", text, Vec2::ZERO, Tone::Quiet)
    })
}

pub fn quiet_toggle(ui: &mut egui::Ui, on: &mut bool, text: &str) -> Response {
    let selected = *on;
    let mut response = tactile(ui, Tone::Quiet, selected, true, |ui| {
        labelled_button(ui, "", text, Vec2::ZERO, Tone::Quiet)
    });
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response
}

pub fn text_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> Response {
    let tone_here = Tone::Neutral;
    tactile(ui, tone_here, false, enabled, |ui| {
        labelled_button(ui, "", text, Vec2::ZERO, tone_here)
    })
}

pub fn primary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> Response {
    let tone_here = Tone::Primary;
    tactile(ui, tone_here, false, enabled, |ui| {
        labelled_button(ui, "", text, Vec2::ZERO, tone_here)
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

/// Whether F11 went down this frame. `key_pressed` fires on key-repeat too, which would retoggle
/// fullscreen every frame while F11 is held; only the initial press should count.
pub fn f11_pressed(ui: &egui::Ui) -> bool {
    ui.input(|i| {
        i.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::F11,
                    pressed: true,
                    repeat: false,
                    ..
                }
            )
        })
    })
}

/// Flips the main window between fullscreen and windowed, from this button or F11. The window opens
/// fullscreen, which hides its title bar, so this is also the way back to its close button.
pub fn fullscreen_button(ui: &mut egui::Ui) {
    let fullscreen = ui.input(|i| i.viewport().fullscreen.unwrap_or(false));
    let (glyph, name) = if fullscreen {
        (icons::WINDOWED, "Exit fullscreen")
    } else {
        (icons::FULLSCREEN, "Fullscreen")
    };
    if icon_button(ui, glyph, name, true).clicked() || f11_pressed(ui) {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
    }
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
        labelled_button(ui, glyph, "", Vec2::splat(CONTROL_HEIGHT), Tone::Neutral)
    });
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, name));
    response.on_hover_text(name)
}

/// A control that stays pressed. Flips `on` and reports the change.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool, glyph: &str, text: &str) -> Response {
    let selected = *on;
    let tone_here = Tone::Neutral;
    let mut response = tactile(ui, tone_here, selected, true, |ui| {
        labelled_button(ui, glyph, text, Vec2::ZERO, tone_here)
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
        labelled_button(ui, glyph, "", Vec2::splat(CONTROL_HEIGHT), Tone::Neutral)
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

/// A choice small enough to sit in a row of text: its value and a chevron, opening the usual menu.
pub fn inline_dropdown(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    text: &str,
    items: impl FnOnce(&mut egui::Ui),
) -> Response {
    const HEIGHT: f32 = 20.0;
    const CHEVRON_ROOM: f32 = 16.0;
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), medium(13.0), secondary());
    let size = egui::vec2(galley.size().x + CHEVRON_ROOM, HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hover = motion::follow(
        ui.ctx(),
        ui.id().with(&id_salt).with("hover"),
        response.hovered() || response.has_focus(),
        motion::HOVER,
    );
    let ink = secondary().lerp_to_gamma(accent(), hover);
    ui.painter().galley(
        egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    paint_chevron(
        ui.painter(),
        egui::pos2(rect.right() - CHEVRON_ROOM / 2.0, rect.center().y),
        3.5,
        2.0,
        ink,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            0,
            Stroke::new(1.0, accent()),
            egui::StrokeKind::Outside,
        );
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, ui.is_enabled(), text));
    egui::Popup::menu(&response).show(items);
    response
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
    ui.painter()
        .rect_stroke(track, 0, Stroke::new(1.0, rule()), egui::StrokeKind::Inside);
    if index.is_some() {
        ui.painter()
            .rect_filled(segment(at).shrink(THUMB_INSET), 0, accent());
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
            if chosen {
                on_accent()
            } else {
                secondary().lerp_to_gamma(self::text(), lit)
            },
        );
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                0,
                Stroke::new(1.0, accent()),
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
        let ink = if ui.is_enabled() {
            self::text()
        } else {
            muted()
        };
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

/// The characters an off/on indicator shows, flipping to reverse video under the pointer.
fn paint_mark(ui: &egui::Ui, response: &Response, glyph: egui::Rect, mark: &str) {
    let ctx = ui.ctx();
    let hover = motion::follow(
        ctx,
        response.id.with("hover"),
        response.hovered(),
        motion::HOVER,
    );
    let ink = if ui.is_enabled() { text() } else { muted() };
    if hover > 0.0 {
        ui.painter()
            .rect_filled(glyph, 0, accent().gamma_multiply(hover));
    }
    ui.painter().text(
        glyph.center(),
        egui::Align2::CENTER_CENTER,
        mark,
        medium(14.0),
        ink.lerp_to_gamma(on_accent(), hover),
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            glyph.expand(2.0),
            0,
            Stroke::new(1.0, accent()),
            egui::StrokeKind::Inside,
        );
    }
}

/// One choice of several, whose dot grows out of the middle when chosen. Sets `current` and reports
/// the change.
pub fn radio<T: PartialEq>(ui: &mut egui::Ui, current: &mut T, value: T, text: &str) -> Response {
    let (mut response, glyph) = indicator_row(ui, egui::vec2(24.0, 16.0), text);
    let mut selected = *current == value;
    if response.clicked() && !selected {
        *current = value;
        response.mark_changed();
        selected = true;
    }
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, enabled, selected, text));
    paint_mark(ui, &response, glyph, if selected { "(*)" } else { "( )" });
    response
}

/// One option that can be on alongside others. Flips `on` and reports the change.
pub fn check(ui: &mut egui::Ui, on: &mut bool, text: &str) -> Response {
    let (mut response, glyph) = indicator_row(ui, egui::vec2(24.0, 16.0), text);
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, *on, text));
    paint_mark(ui, &response, glyph, if *on { "[x]" } else { "[ ]" });
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
    let ink = secondary().lerp_to_gamma(self::text(), hover);
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
            Stroke::new(1.0, accent()),
            egui::StrokeKind::Inside,
        );
    }
    response
}

/// An on/off switch whose thumb travels when flipped and stretches while held. The flip retargets
/// from wherever the thumb is, so a second click mid-travel reverses it without a jump.
pub fn switch(ui: &mut egui::Ui, on: &mut bool, text: &str) -> Response {
    let (mut response, glyph) = indicator_row(ui, egui::vec2(24.0, 16.0), text);
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, *on, text));
    paint_mark(ui, &response, glyph, if *on { "[x]" } else { "[ ]" });
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
    const FRAME: std::time::Duration = std::time::Duration::from_millis(125);
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.ctx().request_repaint_after(FRAME);
    let step = (ui.input(|i| i.time) * 8.0) as usize % 4;
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        ["|", "/", "-", "\\"][step],
        medium(14.0),
        accent().gamma_multiply(opacity),
    );
}

pub fn live_dot(ui: &mut egui::Ui, live: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 16.0), Sense::hover());
    ui.painter().rect_filled(
        egui::Rect::from_center_size(rect.center(), Vec2::splat(7.0)),
        0,
        if live { self::live() } else { muted() },
    );
}

pub fn live_status(ui: &mut egui::Ui, text: &str, live: bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        live_dot(ui, live);
        ui.label(RichText::new(text).color(if live { self::live() } else { secondary() }));
    });
}

pub fn brand(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        crate::icon::paint_mark(ui, 26.0);
        ui.label(
            RichText::new("OpenStream")
                .font(semibold(16.8))
                .color(accent()),
        );
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
        .anchor(egui::Align2::CENTER_CENTER, motion::rise(ctx, shown, RISE));
    let mut closed = false;
    let body = |ui: &mut egui::Ui| {
        ui.set_opacity(shown);
        ui.set_width(inner_width);
        split_row(
            ui,
            CONTROL_HEIGHT,
            |ui| {
                ui.label(RichText::new(title).font(semibold(14.0)));
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

    fn caret_cells(ctx: &egui::Context, time: f64) -> usize {
        let mut text = String::from("Alex");
        let id = egui::Id::new("caret-field");
        let raw = egui::RawInput {
            time: Some(time),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            ui.memory_mut(|memory| memory.request_focus(id));
            field(
                ui,
                egui::TextEdit::singleline(&mut text).id(id),
                CONTROL_HEIGHT,
            );
        });
        output.textures_delta.clear();
        // Counted only where it can be seen: the caret usually sits just past the end of the text.
        output
            .shapes
            .iter()
            .filter(|shape| match &shape.shape {
                egui::Shape::Rect(rect) => {
                    rect.fill == accent()
                        && rect.rect.width() < 12.0
                        && shape.clip_rect.contains_rect(rect.rect)
                }
                _ => false,
            })
            .count()
    }

    #[test]
    fn a_focused_field_shows_a_block_caret_that_blinks() {
        let ctx = egui::Context::default();
        install(&ctx);
        caret_cells(&ctx, 0.0);
        assert_eq!(caret_cells(&ctx, 0.1), 1, "lit");
        assert_eq!(caret_cells(&ctx, 0.7), 0, "off");
        assert_eq!(caret_cells(&ctx, 1.3), 1, "lit again");
    }

    #[test]
    fn asked_for_less_motion_the_caret_holds_still() {
        let ctx = egui::Context::default();
        install(&ctx);
        motion::set_less_motion(&ctx, true);
        caret_cells(&ctx, 0.0);
        assert_eq!(caret_cells(&ctx, 0.7), 1);
    }

    #[test]
    fn a_labels_ink_is_centred_in_its_control() {
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
            assert_eq!(ink.len(), rects.len());
            for (rect, bounds) in rects.iter().zip(&ink) {
                assert!((bounds.center().y - rect.center().y).abs() <= 2.0);
                assert!((bounds.center().x - rect.center().x).abs() <= 2.0);
                assert!(rect.contains_rect(*bounds));
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
    fn every_launch_starts_on_the_blue_tube() {
        assert_eq!(phosphor(), Phosphor::Blue);
        assert_eq!(Phosphor::default(), Phosphor::Blue);
    }

    #[test]
    fn every_text_role_stays_readable_on_every_surface_it_sits_on() {
        for ink in [text(), secondary(), accent(), warning(), error()] {
            for surface in [ground(), surface(), raised()] {
                assert!(contrast(ink, surface) >= 4.5, "{ink:?} on {surface:?}");
            }
        }
        for surface in [ground(), surface()] {
            assert!(contrast(muted(), surface) >= 4.5, "muted on {surface:?}");
        }
        for surface in [accent(), accent_pressed()] {
            assert!(contrast(on_accent(), surface) >= 4.5, "ink on {surface:?}");
        }
        assert!(
            contrast(text(), main_action()) >= 4.5,
            "ink on the main action"
        );
        for ink in [text(), accent()] {
            for surface in [selected(), selected_hover()] {
                assert!(contrast(ink, surface) >= 3.0, "{ink:?} on {surface:?}");
            }
        }
    }

    #[test]
    fn the_interface_font_is_ibm_plex_sans_with_the_bundled_faces_as_fallback() {
        let fonts = fonts();
        let proportional = &fonts.families[&FontFamily::Proportional];
        assert_eq!(proportional[0], "ibm-plex-mono-regular");
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
                assert_eq!(ui.visuals().panel_fill, ground());
            },
        );
        output.textures_delta.clear();
    }
}
