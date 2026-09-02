//! Opt-in update check (Windows). Mirrors InLook's design.
//!
//! - [`maybe_run`] — the auto-check on startup. On first run it asks a one-time
//!   consent question (stored in HKCU); until answered, no network call is made,
//!   so InSearch stays offline-by-default. Only runs if the user opted in.
//! - [`check_now`] — the on-demand check from About → "Check for updates". The
//!   click is its own consent, so it always reports a result and never changes
//!   the auto-check setting.
//! - [`auto_check_enabled`] / [`set_auto_check`] — read/write the auto-check
//!   toggle shown in the About window.
//!
//! It uses **no bundled HTTP or TLS library**: the check goes through the OS's
//! own HTTPS stack (WinHTTP / Schannel). A single redirect-suppressed GET to the
//! public "latest release" URL reads the `Location` header to learn the newest
//! tag, compares it to the running version, and only ever points the user at
//! winget or the releases page — it never downloads or runs anything.

#[cfg(windows)]
pub use imp::{auto_check_enabled, check_now, maybe_run, set_auto_check};

#[cfg(not(windows))]
pub use stub::{auto_check_enabled, check_now, maybe_run, set_auto_check};

/// Semantic-version comparison — pure and testable, no I/O.
mod version {
    struct Version {
        core: (u64, u64, u64),
        is_prerelease: bool,
    }

    fn parse(tag: &str) -> Option<Version> {
        let s = tag.trim().trim_start_matches('v');
        let s = s.split('+').next().unwrap_or(s);
        let (core, pre) = match s.split_once('-') {
            Some((c, _)) => (c, true),
            None => (s, false),
        };
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().unwrap_or("0").parse().ok()?;
        let patch = parts.next().unwrap_or("0").parse().ok()?;
        if parts.next().is_some() {
            return None; // reject "1.2.3.4"
        }
        Some(Version {
            core: (major, minor, patch),
            is_prerelease: pre,
        })
    }

    /// Whether `latest` is a strictly newer release than `current`. `false` on
    /// any unparseable input — we never nag on garbage.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn is_newer(latest: &str, current: &str) -> bool {
        let (Some(l), Some(c)) = (parse(latest), parse(current)) else {
            return false;
        };
        match l.core.cmp(&c.core) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => c.is_prerelease && !l.is_prerelease,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::is_newer;

        #[test]
        fn detects_newer_releases() {
            assert!(is_newer("v0.6.0", "0.5.0"));
            assert!(is_newer("0.5.1", "0.5.0"));
            assert!(is_newer("1.0.0", "0.5.0"));
        }

        #[test]
        fn ignores_same_or_older() {
            assert!(!is_newer("0.5.0", "0.5.0"));
            assert!(!is_newer("0.4.0", "0.5.0"));
        }

        #[test]
        fn prerelease_ranks_below_finished() {
            assert!(is_newer("1.0.0", "1.0.0-rc.1"));
            assert!(!is_newer("1.0.0-rc.2", "1.0.0"));
        }

        #[test]
        fn garbage_never_nags() {
            assert!(!is_newer("", "0.5.0"));
            assert!(!is_newer("banana", "0.5.0"));
        }
    }
}

const RELEASES_URL: &str = "https://github.com/StruisICT/InSearch/releases/latest";

#[cfg(windows)]
mod imp {
    use super::{version, RELEASES_URL};
    use std::ffi::c_void;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Networking::WinHttp::{
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
        WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption,
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_DISABLE_REDIRECTS, WINHTTP_FLAG_SECURE,
        WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_QUERY_LOCATION,
    };
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    const APP_NAME: &str = "InSearch";
    const SETTINGS_KEY: &str = r"Software\StruisICT\InSearch";
    const HOST: PCWSTR = w!("github.com");
    const PATH_LATEST: PCWSTR = w!("/StruisICT/InSearch/releases/latest");
    const HTTPS_PORT: u16 = 443;

    /// The running version (set by build.rs from the release-please manifest).
    fn current() -> &'static str {
        env!("INSEARCH_VERSION")
    }

    fn setting(name: &str) -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(SETTINGS_KEY)
            .ok()?
            .get_value::<String, _>(name)
            .ok()
    }

    fn put_setting(name: &str, value: &str) {
        if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(SETTINGS_KEY) {
            let _ = k.set_value(name, &value.to_string());
        }
    }

    fn prompt_answered() -> bool {
        setting("UpdateCheckPrompted").as_deref() == Some("1")
    }

    /// Whether auto-checks are on (for the About toggle).
    pub fn auto_check_enabled() -> bool {
        setting("UpdateCheckEnabled").as_deref() == Some("1")
    }

    /// Set the auto-check toggle (also marks the one-time prompt as answered).
    pub fn set_auto_check(enabled: bool) {
        put_setting("UpdateCheckPrompted", "1");
        put_setting("UpdateCheckEnabled", if enabled { "1" } else { "0" });
    }

    /// Startup auto-check: prompt once for consent, then (if enabled) check and
    /// announce a newer version at most once. Runs **entirely on a background
    /// thread** — the one-time consent dialog and the network call must never
    /// block the UI thread (a modal shown from inside the egui frame would hide
    /// behind the window and freeze input).
    pub fn maybe_run() {
        std::thread::spawn(|| {
            if !prompt_answered() {
                let enabled = ask_consent();
                set_auto_check(enabled);
                if !enabled {
                    return;
                }
            }
            if !auto_check_enabled() {
                return;
            }
            let Some(tag) = fetch_latest_tag() else {
                return;
            };
            if !version::is_newer(&tag, current()) {
                return;
            }
            let normalized = tag.trim_start_matches('v').to_string();
            if setting("LastNotifiedVersion").as_deref() == Some(normalized.as_str()) {
                return; // already announced this version
            }
            put_setting("LastNotifiedVersion", &normalized);
            notify_update_available(&normalized);
        });
    }

    /// On-demand check from About. Always reports a result; its own consent.
    pub fn check_now() {
        use rfd::{MessageButtons, MessageDialog, MessageLevel};
        std::thread::spawn(|| match fetch_latest_tag() {
            Some(tag) if version::is_newer(&tag, current()) => {
                notify_update_available(tag.trim_start_matches('v'));
            }
            Some(_) => {
                MessageDialog::new()
                    .set_level(MessageLevel::Info)
                    .set_title(APP_NAME)
                    .set_description(format!(
                        "You're on the latest version (InSearch {}).",
                        current()
                    ))
                    .set_buttons(MessageButtons::Ok)
                    .show();
            }
            None => {
                let open = matches!(
                    MessageDialog::new()
                        .set_level(MessageLevel::Warning)
                        .set_title(APP_NAME)
                        .set_description(
                            "Couldn't check for updates right now.\n\n\
                             Open the releases page to check manually?",
                        )
                        .set_buttons(MessageButtons::YesNo)
                        .show(),
                    rfd::MessageDialogResult::Yes
                );
                if open {
                    crate::reveal::open_url(RELEASES_URL);
                }
            }
        });
    }

    /// One-time consent dialog. Cancel/close is treated as "no" (stay offline).
    fn ask_consent() -> bool {
        use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
        matches!(
            MessageDialog::new()
                .set_level(MessageLevel::Info)
                .set_title(APP_NAME)
                .set_description(
                    "Check for InSearch updates automatically?\n\n\
                     InSearch is offline by default. If you choose Yes, it will \
                     occasionally contact github.com (over HTTPS, using Windows' \
                     own secure connection) to see whether a newer version exists. \
                     It never downloads or installs anything automatically, and \
                     sends no information about you or your searches.\n\n\
                     Either way, you can check any time from About \u{2192} \
                     \"Check for updates\".",
                )
                .set_buttons(MessageButtons::YesNo)
                .show(),
            MessageDialogResult::Yes
        )
    }

    /// Tell the user a newer version exists; "Yes" opens the releases page.
    fn notify_update_available(latest: &str) {
        use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
        let result = MessageDialog::new()
            .set_level(MessageLevel::Info)
            .set_title(APP_NAME)
            .set_description(format!(
                "InSearch {latest} is available (you have {}).\n\n\
                 To update:\n\
                 \u{2022} winget:  winget upgrade StruisICT.InSearch\n\
                 \u{2022} or download it from the releases page.\n\n\
                 Open the releases page now?",
                current()
            ))
            .set_buttons(MessageButtons::YesNo)
            .show();
        if matches!(result, MessageDialogResult::Yes) {
            crate::reveal::open_url(RELEASES_URL);
        }
    }

    /// Ask GitHub for the "latest" redirect and read the `Location` header (e.g.
    /// `.../releases/tag/v0.6.0`). `None` on any failure — best-effort, silent.
    fn fetch_latest_tag() -> Option<String> {
        struct Handle(*mut c_void);
        impl Drop for Handle {
            fn drop(&mut self) {
                if !self.0.is_null() {
                    unsafe {
                        let _ = WinHttpCloseHandle(self.0);
                    }
                }
            }
        }

        unsafe {
            let session = Handle(WinHttpOpen(
                w!("InSearch-update-check"),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ));
            if session.0.is_null() {
                return None;
            }
            let connect = Handle(WinHttpConnect(session.0, HOST, HTTPS_PORT, 0));
            if connect.0.is_null() {
                return None;
            }
            let request = Handle(WinHttpOpenRequest(
                connect.0,
                w!("GET"),
                PATH_LATEST,
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ));
            if request.0.is_null() {
                return None;
            }

            // Suppress auto-redirect so we can read the 302's Location ourselves.
            WinHttpSetOption(
                Some(request.0),
                WINHTTP_OPTION_DISABLE_FEATURE,
                Some(&WINHTTP_DISABLE_REDIRECTS.to_le_bytes()),
            )
            .ok()?;

            WinHttpSendRequest(request.0, None, None, 0, 0, 0).ok()?;
            WinHttpReceiveResponse(request.0, std::ptr::null_mut()).ok()?;

            let mut buf = [0u16; 512];
            let mut len = (buf.len() * std::mem::size_of::<u16>()) as u32;
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_LOCATION,
                PCWSTR::null(),
                Some(buf.as_mut_ptr() as *mut c_void),
                &mut len,
                std::ptr::null_mut(),
            )
            .ok()?;

            let n = (len as usize) / std::mem::size_of::<u16>();
            tag_from_location(&String::from_utf16_lossy(&buf[..n]))
        }
    }

    /// Extract the version tag from a `.../releases/tag/<tag>` URL. Pure.
    fn tag_from_location(location: &str) -> Option<String> {
        let tag = location.trim_end_matches('/').rsplit('/').next()?;
        if tag.is_empty() || !tag.starts_with('v') {
            return None;
        }
        Some(tag.to_string())
    }

    #[cfg(test)]
    mod tests {
        use super::tag_from_location;

        #[test]
        fn extracts_tag_from_github_redirect() {
            assert_eq!(
                tag_from_location("https://github.com/StruisICT/InSearch/releases/tag/v0.6.0"),
                Some("v0.6.0".to_string())
            );
            assert_eq!(
                tag_from_location("https://github.com/StruisICT/InSearch/releases/tag/v1.0.0/"),
                Some("v1.0.0".to_string())
            );
        }

        #[test]
        fn rejects_unexpected_locations() {
            assert_eq!(tag_from_location("https://github.com/login"), None);
            assert_eq!(tag_from_location(""), None);
            assert_eq!(
                tag_from_location("https://github.com/StruisICT/InSearch/releases"),
                None
            );
        }
    }
}

#[cfg(not(windows))]
mod stub {
    /// No auto-check off Windows; just open the releases page on demand.
    pub fn maybe_run() {}

    pub fn check_now() {
        crate::reveal::open_url(super::RELEASES_URL);
    }

    pub fn auto_check_enabled() -> bool {
        false
    }

    pub fn set_auto_check(_enabled: bool) {}
}
