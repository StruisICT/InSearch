//! Colour palette and global style, modelled on ClutterCutter's egui theme:
//! a warm-white light mode and a soft dark mode, both with a brand-blue accent,
//! subtle hairlines, rounded widgets and comfortable spacing.

use eframe::egui::{self, Color32};

pub struct Pal {
    pub win_bg: Color32,
    pub panel_bg: Color32,
    pub card_bg: Color32,
    pub card_sel: Color32,
    pub text: Color32,
    pub subtext: Color32,
    pub hairline: Color32,
    pub track: Color32,
    pub blue: Color32,
}

pub fn palette(dark: bool) -> Pal {
    if dark {
        Pal {
            win_bg: Color32::from_rgb(0x1C, 0x1E, 0x22),
            panel_bg: Color32::from_rgb(0x22, 0x25, 0x2A),
            card_bg: Color32::from_rgb(0x2A, 0x2E, 0x34),
            card_sel: Color32::from_rgb(0x33, 0x39, 0x42),
            text: Color32::from_rgb(0xE6, 0xE9, 0xEC),
            subtext: Color32::from_rgb(0x9A, 0xA2, 0xAC),
            hairline: Color32::from_rgb(0x3A, 0x40, 0x48),
            track: Color32::from_rgb(0x34, 0x3A, 0x42),
            blue: Color32::from_rgb(0x4C, 0x8B, 0xFF),
        }
    } else {
        // Warm-white light theme.
        Pal {
            win_bg: Color32::from_rgb(0xF0, 0xEB, 0xE3),
            panel_bg: Color32::from_rgb(0xF6, 0xF1, 0xE8),
            card_bg: Color32::from_rgb(0xFB, 0xF8, 0xF2),
            card_sel: Color32::from_rgb(0xEE, 0xF3, 0xFC),
            text: Color32::from_rgb(0x26, 0x23, 0x20),
            subtext: Color32::from_rgb(0x8C, 0x83, 0x78),
            hairline: Color32::from_rgb(0xE8, 0xE1, 0xD6),
            track: Color32::from_rgb(0xED, 0xE6, 0xDB),
            blue: Color32::from_rgb(0x2D, 0x6B, 0xF0),
        }
    }
}

/// Push the palette and a global style (rounding, spacing, fonts) into egui.
pub fn apply(ctx: &egui::Context, dark: bool) {
    let p = palette(dark);

    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.override_text_color = Some(p.text);
    v.panel_fill = p.win_bg;
    v.window_fill = p.panel_bg;
    v.extreme_bg_color = p.track; // text-input background
    v.hyperlink_color = p.blue;
    v.selection.bg_fill = p.blue.linear_multiply(0.35);
    v.selection.stroke = egui::Stroke::new(1.0, p.blue);
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, p.hairline);

    // Soft, rounded widgets that sit on the card colour.
    let cr = egui::CornerRadius::same(5);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = cr;
    }
    v.widgets.inactive.weak_bg_fill = p.card_bg;
    v.widgets.inactive.bg_fill = p.card_bg;
    v.widgets.hovered.weak_bg_fill = p.card_sel;
    v.widgets.hovered.bg_fill = p.card_sel;
    v.window_corner_radius = egui::CornerRadius::same(8);
    v.menu_corner_radius = egui::CornerRadius::same(6);
    ctx.set_visuals(v);

    // Comfortable spacing + slightly larger, calmer type scale (applied to both
    // the light and dark styles).
    use egui::{FontFamily::Proportional, FontId, TextStyle};
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.interact_size.y = 24.0;
        style.spacing.window_margin = egui::Margin::same(10);
        style.text_styles = [
            (TextStyle::Heading, FontId::new(20.0, Proportional)),
            (TextStyle::Body, FontId::new(14.0, Proportional)),
            (TextStyle::Button, FontId::new(14.0, Proportional)),
            (
                TextStyle::Monospace,
                FontId::new(13.0, egui::FontFamily::Monospace),
            ),
            (TextStyle::Small, FontId::new(11.0, Proportional)),
        ]
        .into();
    });
}
