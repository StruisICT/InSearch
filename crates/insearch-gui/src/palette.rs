//! Minimal light/dark theming. Kept tiny for now; expand later with a custom
//! palette if desired.

use eframe::egui;

pub fn apply(ctx: &egui::Context, dark: bool) {
    let visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    ctx.set_visuals(visuals);
}
