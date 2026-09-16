//! InSearch — egui desktop front-end.
//!
//! Launch with an optional path argument (used by the Windows Explorer
//! "Search with InSearch" entry) to prefill the search root:
//! `insearch-gui "C:\logs"`.
//!
//! Renderer flags (see `renderer.rs`): `--renderer glow|wgpu`, `--software-gpu`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod context_menu;
mod palette;
mod renderer;
mod reveal;
mod session;
mod update;

/// App icon, used for the window/taskbar (cross-platform). On Windows the same
/// artwork is also embedded in the exe via `app.rc` for Explorer/context menu.
const ICON_PNG: &[u8] = include_bytes!("../icon-256.png");

use std::process::ExitCode;

use eframe::egui;

fn main() -> ExitCode {
    let args = renderer::parse_args();
    let choice = args.renderer;
    let initial_root = args.root;

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1080.0, 700.0])
        .with_min_inner_size([720.0, 460.0])
        .with_title("InSearch");
    if let Ok(icon) = eframe::icon_data::from_png_bytes(ICON_PNG) {
        viewport = viewport.with_icon(icon);
    }

    let mut native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    if let Err(err) = renderer::apply(&choice, &mut native_options) {
        eprintln!("InSearch: {err}");
        return ExitCode::FAILURE;
    }

    match eframe::run_native(
        "InSearch",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, initial_root.clone())))),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!(
                "InSearch could not open a window with the {} renderer: {err}",
                choice.backend.name()
            );

            // The default wgpu path (DirectX 12, WARP fallback) should start
            // anywhere Windows has a desktop. If it still failed, give the
            // OpenGL backend one shot in a fresh process — winit refuses to
            // create a second event loop in this one. An explicit choice is
            // respected as-is.
            if choice.backend == renderer::Backend::Wgpu && !choice.explicit {
                eprintln!("InSearch: retrying with the glow (OpenGL) renderer…");
                return relaunch_with_glow(&args.passthrough);
            }

            // Reaching here means no windowing/graphics context could be
            // created at all — a headless session, a service context, or a
            // broken driver. That's an environment limitation, not a fault in
            // InSearch, so report it and exit cleanly rather than propagate a
            // non-zero code. (Propagating would also trip winget's install-time
            // executable validation, which launches the exe on a GPU-less runner.)
            eprintln!(
                "This usually means no display is available (headless server or service \
                 session) or the graphics driver is broken. Try `--software-gpu` or \
                 `--renderer glow`; the `insearch-cli` tool works without a display."
            );
            ExitCode::SUCCESS
        }
    }
}

/// Re-run this executable with `--renderer glow` plus the original non-renderer
/// arguments, and mirror its exit status.
fn relaunch_with_glow(passthrough: &[String]) -> ExitCode {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("InSearch: cannot locate own executable for relaunch: {err}");
            return ExitCode::SUCCESS;
        }
    };
    match std::process::Command::new(exe)
        .arg("--renderer")
        .arg("glow")
        .args(passthrough)
        .status()
    {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("InSearch: glow relaunch exited with {status}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("InSearch: glow relaunch failed to start: {err}");
            ExitCode::SUCCESS
        }
    }
}
