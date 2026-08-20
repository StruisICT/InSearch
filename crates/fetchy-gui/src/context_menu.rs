//! Windows Explorer "Search with Inspector Fetchy" context-menu integration.
//!
//! Registration lives entirely under `HKCU\Software\Classes` — no admin rights,
//! no machine-wide changes. Three verbs are installed so the entry appears when
//! right-clicking a file, a folder, or the background of an open folder. On
//! Windows 11 the classic entry shows under "Show more options" (a modern
//! `IExplorerCommand` menu is a possible later enhancement).
//!
//! This module only *touches* the registry when the user invokes [`register`]
//! or [`unregister`] from the in-app Settings panel.

pub use imp::{register, status, unregister};

/// Whether the integration is installed, and whether it still points here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// No context-menu entry installed.
    NotRegistered,
    /// Installed and pointing at this executable.
    Registered,
    /// Installed but pointing at a different (moved/old) executable.
    Stale,
    /// Could not determine (e.g. failed to read the current exe path).
    Error,
}

#[cfg(windows)]
mod imp {
    pub use super::Status;
    use std::io;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    const LABEL: &str = "Search with Inspector Fetchy";

    /// (verb key path under HKCU, argument placeholder for the command).
    /// `%1` = the selected file/folder; `%V` = the open folder's background.
    const VERBS: [(&str, &str); 3] = [
        (r"Software\Classes\*\shell\InspectorFetchy", "%1"),
        (r"Software\Classes\Directory\shell\InspectorFetchy", "%1"),
        (
            r"Software\Classes\Directory\Background\shell\InspectorFetchy",
            "%V",
        ),
    ];

    fn exe_path() -> io::Result<String> {
        Ok(std::env::current_exe()?.to_string_lossy().into_owned())
    }

    /// Install the three verbs pointing at the current executable.
    pub fn register() -> io::Result<()> {
        let exe = exe_path()?;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for (base, arg) in VERBS {
            let (verb, _) = hkcu.create_subkey(base)?;
            verb.set_value("", &LABEL.to_string())?;
            verb.set_value("Icon", &format!("\"{exe}\""))?;
            let (command, _) = hkcu.create_subkey(format!(r"{base}\command"))?;
            command.set_value("", &format!("\"{exe}\" \"{arg}\""))?;
        }
        Ok(())
    }

    /// Remove the three verbs (and their `command` subkeys). Missing keys are
    /// ignored so this is safe to call when nothing is installed.
    pub fn unregister() -> io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for (base, _) in VERBS {
            let _ = hkcu.delete_subkey_all(base);
        }
        Ok(())
    }

    /// Report whether the integration is installed and still points here.
    pub fn status() -> Status {
        let exe = match exe_path() {
            Ok(e) => e,
            Err(_) => return Status::Error,
        };
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let mut present = 0;
        let mut stale = false;
        for (base, _) in VERBS {
            if let Ok(command) = hkcu.open_subkey(format!(r"{base}\command")) {
                present += 1;
                let value: String = command.get_value("").unwrap_or_default();
                if !value.contains(&exe) {
                    stale = true;
                }
            }
        }
        if present == 0 {
            Status::NotRegistered
        } else if stale || present < VERBS.len() {
            Status::Stale
        } else {
            Status::Registered
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub use super::Status;

    pub fn register() -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "the Explorer context menu is Windows-only",
        ))
    }

    pub fn unregister() -> std::io::Result<()> {
        Ok(())
    }

    pub fn status() -> Status {
        Status::NotRegistered
    }
}
