//! InSearch — egui desktop front-end.
//!
//! Launch with an optional path argument (used by the Windows Explorer
//! "Search with InSearch" entry, added in a later phase) to prefill the
//! search root: `insearch-gui "C:\logs"`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod context_menu;
mod palette;
mod reveal;
mod session;

/// App icon, used for the window/taskbar (cross-platform). On Windows the same
/// artwork is also embedded in the exe via `app.rc` for Explorer/context menu.
const ICON_PNG: &[u8] = include_bytes!("../icon-256.png");

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // First non-flag argument is an initial search root, if any.
    let initial_root: Option<PathBuf> = std::env::args()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .map(PathBuf::from);

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1080.0, 700.0])
        .with_min_inner_size([720.0, 460.0])
        .with_title("InSearch");
    if let Ok(icon) = eframe::icon_data::from_png_bytes(ICON_PNG) {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "InSearch",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, initial_root.clone())))),
    )
}
