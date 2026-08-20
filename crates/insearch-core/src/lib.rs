//! InSearch — content-aware file search engine (headless core).
//!
//! Pipeline: [`scan`] walks roots and streams matches; [`extract`] decides how a
//! file becomes text (raw vs. decoded); [`split`] divides text into line/block
//! units. The GUI and CLI are thin front-ends over this crate.

pub mod extract;
pub mod model;
pub mod scan;
pub mod split;
pub mod watch;

// Everyday API. The extension surface (extractors, splitters) stays under its
// modules — `extract::{TextExtractor, Registry}`, `split::{UnitSplitter, ...}` —
// so this prelude reflects what front-ends actually use.
pub use model::{CaseMode, Granularity, Match, MatchMode, Mode, Query, SearchEvent};
pub use scan::{highlight_regex, search, search_collect, FileFilter, ScanOptions};
pub use watch::{start as start_watch, WatchHandle};
