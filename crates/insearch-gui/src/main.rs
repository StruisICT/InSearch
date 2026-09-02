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
mod update;

/// App icon, used for the window/taskbar (cross-platform). On Windows the same
/// artwork is also embedded in the exe via `app.rc` for Explorer/context menu.
const ICON_PNG: &[u8] = include_bytes!("../icon-256.png");

use std::path::PathBuf;
use std::process::ExitCode;

use eframe::egui;

fn main() -> ExitCode {
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

    match eframe::run_native(
        "InSearch",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, initial_root.clone())))),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Reaching here almost always means the windowing / OpenGL (glow) context
            // failed to initialise — a headless VM, an RDP session without hardware
            // acceleration, or a stale graphics driver. That's an environment
            // limitation, not a fault in InSearch, so report it and exit cleanly
            // rather than propagating a non-zero code. (Propagating would also trip
            // winget's install-time executable validation, which launches the exe on
            // a GPU-less runner.)
            eprintln!(
                "InSearch could not open a window: {err}\n\
                 This usually means no GPU/display is available (headless server, RDP \
                 without hardware acceleration, or an outdated graphics driver)."
            );
            ExitCode::SUCCESS
        }
    }
}
