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

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // First non-flag argument is an initial search root, if any.
    let initial_root: Option<PathBuf> = std::env::args()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .map(PathBuf::from);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1080.0, 700.0])
            .with_min_inner_size([720.0, 460.0])
            .with_title("InSearch"),
        ..Default::default()
    };

    eframe::run_native(
        "InSearch",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, initial_root.clone())))),
    )
}
