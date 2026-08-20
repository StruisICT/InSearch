//! Persisted session state: recent queries and saved searches.
//!
//! Stored as JSON under the platform config dir (`%APPDATA%\InSearch`,
//! `~/.config/InSearch`, or `~/Library/Application Support/InSearch`). All I/O
//! is best-effort — a missing or unreadable file just yields an empty session.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// How many recent queries to remember.
const MAX_RECENT: usize = 20;

/// A saved search: the query options plus roots, keyed by a user-given name.
/// Enum options are stored as small integers so this file stays decoupled from
/// the core's enum definitions.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct SavedSearch {
    pub name: String,
    pub query: String,
    pub exclude: String,
    pub match_mode: u8,
    pub case: u8,
    pub whole_word: bool,
    pub granularity: u8,
    pub roots: Vec<PathBuf>,
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
