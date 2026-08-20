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

pub use model::{Granularity, Match, Mode, Query, SearchEvent};
pub use scan::{search, search_collect, ScanOptions};
pub use watch::{start as start_watch, WatchHandle};
