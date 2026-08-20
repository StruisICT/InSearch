//! Shared data types for the search engine.

use std::path::PathBuf;

/// How a file's text is divided into searchable units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Granularity {
    /// One unit per line (the fast path, backed by grep-searcher).
    #[default]
    Line,
    /// One unit per timestamp-to-timestamp block (see `split::BlockSplitter`).
    Block,
}

/// Live search-as-you-type vs. watch-folders (log-tailing) mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Live,
    Watch,
}

/// A user query plus its matching options.
#[derive(Clone, Debug)]
pub struct Query {
    /// The raw pattern. Treated as a literal substring unless `is_regex`.
    pub pattern: String,
    /// Interpret `pattern` as a regular expression.
    pub is_regex: bool,
    /// ripgrep-style smart case: case-insensitive unless the pattern has an
    /// uppercase letter.
    pub smart_case: bool,
    /// Report matches per-line or per-block.
    pub granularity: Granularity,
}

impl Query {
    pub fn literal(pattern: impl Into<String>) -> Self {
        Query {
            pattern: pattern.into(),
            is_regex: false,
            smart_case: true,
            granularity: Granularity::Line,
        }
    }
}

/// A single search hit.
#[derive(Clone, Debug)]
pub struct Match {
    pub path: PathBuf,
    /// 1-based line number where the matched unit starts.
    pub line_start: u64,
    /// 1-based line number where the matched unit ends (== `line_start` for
    /// line mode).
    pub line_end: u64,
    /// Byte offset of the unit start within the file (for jump-to / tailing).
    /// Approximate for non-UTF-8 input, which is decoded lossily first.
    pub byte_offset: u64,
    /// The unit text (single line, or whole block), possibly capped for display.
    pub text: String,
    /// The specific line within a block that matched (== `line_start` for line
    /// mode).
    pub matched_line: u64,
}

/// Streamed over the results channel from a worker to the UI/CLI.
///
/// Every event carries the `generation` of the search that produced it so the
/// consumer can drop results from a superseded (stale) search.
#[derive(Clone, Debug)]
pub enum SearchEvent {
    /// A hit (the leading `u64` is the generation, as for every variant).
    Match(u64, Match),
    /// Drop all existing matches for this path (watch mode: a file changed,
    /// was removed, or is about to be fully re-scanned). No-op if none exist.
    Clear(u64, std::path::PathBuf),
    /// The walk finished (or was cancelled) for this generation.
    Done(u64),
    /// A fatal error (e.g. an invalid regex) for this generation.
    Error(u64, String),
}
