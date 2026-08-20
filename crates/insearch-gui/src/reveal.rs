//! Open a file with the OS default application, or reveal it in the file
//! manager. Platform-specific, best-effort (errors are ignored — these are
//! convenience actions).

use std::path::Path;
use std::process::Command;

/// Open `path` with the default application.
pub fn open_path(path: &Path) {
    #[cfg(windows)]
    {
        // `cmd /C start "" "<path>"` — the empty "" is start's window-title arg.
        let _ = Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
}

/// Reveal `path` in the OS file manager (selecting it where supported).
pub fn reveal(path: &Path) {
    #[cfg(windows)]
    {
        let _ = Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
    #[cfg(not(windows))]
    {
        // No portable "select" on Linux/macOS; open the containing folder.
        let dir = path.parent().unwrap_or(path);
        #[cfg(target_os = "macos")]
        let _ = Command::new("open").arg(dir).spawn();
        #[cfg(all(unix, not(target_os = "macos")))]
        let _ = Command::new("xdg-open").arg(dir).spawn();
    }
}
