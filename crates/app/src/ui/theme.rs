//! OpenStream's dark viewing-room material, shared by every native window.
use egui::{Color32, FontId, Margin, Stroke, TextStyle, Vec2};

pub const GROUND: Color32 = Color32::from_rgb(9, 9, 11);
pub const SURFACE: Color32 = Color32::from_rgb(19, 19, 22);
pub const RAISED: Color32 = Color32::from_rgb(31, 31, 35);
pub const TEXT: Color32 = Color32::from_rgb(242, 242, 245);
pub const SECONDARY: Color32 = Color32::from_rgb(169, 169, 179);
pub const LIVE: Color32 = Color32::from_rgb(113, 214, 164);
pub const ERROR: Color32 = Color32::from_rgb(245, 143, 151);
pub const WARNING: Color32 = Color32::from_rgb(230, 190, 121);

pub fn install(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_global_style(style());
}

fn style() -> egui::Style {
    let mut style = egui::Style {
        text_styles: [
            (TextStyle::Heading, FontId::proportional(20.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into(),
        animation_time: 0.12,
        ..Default::default()
    };
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 8.0);
    style.spacing.interact_size = Vec2::new(36.0, 32.0);
    style.spacing.window_margin = Margin::same(24);
    style.spacing.menu_margin = Margin::same(8);
    style.spacing.slider_width = 96.0;
    style.spacing.combo_width = 112.0;
    style.spacing.indent = 16.0;
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = GROUND;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = GROUND;
    visuals.text_edit_bg_color = Some(GROUND);
    visuals.faint_bg_color = SURFACE;
    visuals.code_bg_color = RAISED;
    visuals.weak_text_color = Some(SECONDARY);
    visuals.window_highlight_topmost = false;
    visuals.window_corner_radius = 16.into();
    visuals.menu_corner_radius = 8.into();
    visuals.window_stroke = Stroke::new(1.0, Color32::from_gray(48));
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(80),
    };
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = ERROR;
    visuals.hyperlink_color = TEXT;
    visuals.selection.bg_fill = Color32::from_gray(65);
    visuals.selection.stroke = Stroke::new(1.0, TEXT);
    for (widget, fill, border, ink) in [
        (
            &mut visuals.widgets.noninteractive,
            SURFACE,
            Color32::from_gray(42),
            TEXT,
        ),
        (
            &mut visuals.widgets.inactive,
            RAISED,
            Color32::from_gray(55),
            TEXT,
        ),
        (
            &mut visuals.widgets.hovered,
            Color32::from_gray(48),
            Color32::from_gray(96),
            TEXT,
        ),
        (
            &mut visuals.widgets.active,
            Color32::from_gray(58),
            TEXT,
            TEXT,
        ),
        (
            &mut visuals.widgets.open,
            Color32::from_gray(48),
            Color32::from_gray(96),
            TEXT,
        ),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0, border);
        widget.fg_stroke = Stroke::new(1.5, ink);
        widget.corner_radius = 8.into();
        widget.expansion = 0.0;
    }
    style.visuals = visuals;
    style
}

pub fn panel() -> egui::Frame {
    egui::Frame::NONE.fill(SURFACE).inner_margin(16)
}

pub fn card() -> egui::Frame {
    panel().corner_radius(16).inner_margin(24)
}

pub fn primary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for (widget, fill) in [
            (&mut widgets.inactive, TEXT),
            (&mut widgets.hovered, Color32::WHITE),
            (&mut widgets.active, Color32::from_gray(196)),
        ] {
            widget.weak_bg_fill = fill;
            widget.bg_fill = fill;
            widget.fg_stroke.color = GROUND;
        }
        ui.add_enabled(
            enabled,
            egui::Button::new(text).min_size(egui::vec2(0.0, 36.0)),
        )
    })
    .inner
}

pub fn live_status(ui: &mut egui::Ui, text: &str, live: bool) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 16.0), egui::Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), 3.0, if live { LIVE } else { SECONDARY });
        ui.label(egui::RichText::new(text).color(if live { LIVE } else { SECONDARY }));
    });
}

pub fn brand(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
        let back =
            egui::Rect::from_min_size(rect.min + egui::vec2(1.0, 3.0), egui::vec2(20.0, 15.0));
        let front = back.translate(egui::vec2(6.0, 6.0));
        for screen in [back, front] {
            ui.painter().rect_filled(screen, 3.0, SURFACE);
            ui.painter().rect_stroke(
                screen,
                3.0,
                Stroke::new(1.5, TEXT),
                egui::StrokeKind::Inside,
            );
        }
        ui.label(egui::RichText::new("OpenStream").size(20.0));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
