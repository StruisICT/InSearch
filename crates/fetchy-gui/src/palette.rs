//! Minimal light/dark theming. Kept tiny for the MVP; expand later to match
//! ClutterCutter's warm-white palette if desired.

use eframe::egui;

pub fn apply(ctx: &egui::Context, dark: bool) {
    let visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    ctx.set_visuals(visuals);
}
