//! Persisted session state: recent queries and saved searches.
//!
//! Stored as JSON under the platform config dir (`%APPDATA%\InSearch`,
//! `~/.config/InSearch`, or `~/Library/Application Support/InSearch`). All I/O
//! is best-effort — a missing or unreadable file just yields an empty session.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// How many recent queries to remember.
const MAX_RECENT: usize = 20;

/// A saved search: the query options, filters, and roots, keyed by a user-given
/// name. Enum options are stored as small integers so this file stays decoupled
/// from the core's enum definitions. `#[serde(default)]` lets searches saved by
/// older versions (without the filter fields) still load.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct SavedSearch {
    pub name: String,
    pub query: String,
    pub exclude: String,
    pub match_mode: u8,
    pub case: u8,
    pub whole_word: bool,
    pub granularity: u8,
    pub roots: Vec<PathBuf>,
    // Filters (added later). All raw UI strings/flags, so restoring is a plain
    // assignment and empty means "no restriction".
    pub filter_name: String,
    pub filter_name_regex: bool,
    pub filter_include_exts: String,
    pub filter_exclude_exts: String,
    pub filter_min_kb: String,
    pub filter_max_kb: String,
    pub filter_days: String,
    pub filter_after: String,
    pub filter_before: String,
    pub filter_ts_after: String,
    pub filter_ts_before: String,
    pub ts_mtime_prefilter: bool,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Session {
    pub recent_queries: Vec<String>,
    pub saved: Vec<SavedSearch>,
}

impl Session {
    /// Load the session, or an empty one if none exists / can't be read.
    pub fn load() -> Self {
        let Some(path) = session_path() else {
            return Session::default();
        };
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persist to disk (best-effort).
    pub fn save(&self) {
        let Some(path) = session_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }

    /// Record a query at the front of the recents (deduplicated, capped), then
    /// persist. No-op for blank queries.
    pub fn record_query(&mut self, query: &str) {
        let q = query.trim();
        if q.is_empty() {
            return;
        }
        self.recent_queries.retain(|existing| existing != q);
        self.recent_queries.insert(0, q.to_string());
        self.recent_queries.truncate(MAX_RECENT);
        self.save();
    }

    /// Add or replace a saved search by name, then persist.
    pub fn add_saved(&mut self, entry: SavedSearch) {
        self.saved.retain(|s| s.name != entry.name);
        self.saved.push(entry);
        self.save();
    }

    /// Remove a saved search by name, then persist.
    pub fn remove_saved(&mut self, name: &str) {
        self.saved.retain(|s| s.name != name);
        self.save();
    }
}

/// Path to the session JSON file, or `None` if no config dir can be resolved.
fn session_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("InSearch").join("session.json"))
}

/// The platform per-user config directory.
fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_search_roundtrips_filters() {
        let s = SavedSearch {
            name: "n".into(),
            query: "q".into(),
            filter_name: "*.log".into(),
            filter_ts_after: "2026-08-21".into(),
            ts_mtime_prefilter: true,
            ..SavedSearch::default()
        };
        let back: SavedSearch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.filter_name, "*.log");
        assert_eq!(back.filter_ts_after, "2026-08-21");
        assert!(back.ts_mtime_prefilter);
    }

    #[test]
    fn old_saved_search_without_filters_still_loads() {
        // A pre-filters JSON blob — the filter fields are absent entirely.
        let json = r#"{"name":"n","query":"q","exclude":"","match_mode":0,
            "case":0,"whole_word":false,"granularity":0,"roots":[]}"#;
        let s: SavedSearch = serde_json::from_str(json).unwrap();
        assert_eq!(s.query, "q");
        assert_eq!(s.filter_name, ""); // defaulted
        assert!(!s.ts_mtime_prefilter);
    }
}
