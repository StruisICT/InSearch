//! The streaming search engine.
//!
//! [`search`] walks the given roots with ripgrep's parallel walker and searches
//! each file *inside the walker's own thread pool* (one fused pool — no
//! second rayon pool per file). Matches stream out over a bounded channel as
//! [`SearchEvent`]s tagged with a `generation`.
//!
//! Cancellation is a generation counter: the caller bumps a shared `AtomicU64`
//! to start a new search; every stale worker notices the mismatch at the next
//! file (or the next result) and quits on its own, so a fast typist never has to
//! join the previous run.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crossbeam_channel::Sender;
use grep_matcher::Matcher as _;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkMatch};
use ignore::{WalkBuilder, WalkState};

use crate::extract::{default_registry, Source};
use crate::model::{Granularity, Match, Query, SearchEvent};
use crate::split::{BlockSplitter, Unit, UnitSplitter};

/// Cap block text length (in characters) held per match for display.
const BLOCK_DISPLAY_CAP: usize = 2000;

/// Options controlling the walk. Sensible defaults for a document/log searcher:
/// see everything (don't honour `.gitignore`, do descend hidden dirs).
#[derive(Clone, Copy, Debug)]
pub struct ScanOptions {
    pub respect_gitignore: bool,
    pub include_hidden: bool,
    pub follow_links: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions {
            respect_gitignore: false,
            include_hidden: true,
            follow_links: false,
        }
    }
}

/// Build a grep matcher from a query. Literal queries are regex-escaped.
pub fn build_matcher(query: &Query) -> Result<RegexMatcher, String> {
    let pattern = if query.is_regex {
        query.pattern.clone()
    } else {
        regex::escape(&query.pattern)
    };
    RegexMatcherBuilder::new()
        .case_smart(query.smart_case)
        .build(&pattern)
        .map_err(|e| e.to_string())
}

/// Run a search to completion (blocking — call from a worker thread).
///
/// Streams [`SearchEvent::Match`] as hits are found and a final
/// [`SearchEvent::Done`] (or [`SearchEvent::Error`]). All events carry
/// `generation`; the consumer drops those that don't match the active search.
pub fn search(
    roots: &[PathBuf],
    query: &Query,
    generation: u64,
    current_gen: Arc<AtomicU64>,
    opts: ScanOptions,
    tx: Sender<SearchEvent>,
) {
    let matcher = match build_matcher(query) {
        Ok(m) => m,
        Err(e) => {
            let _ = tx.send(SearchEvent::Error(generation, e));
            return;
        }
    };

    if roots.is_empty() {
        let _ = tx.send(SearchEvent::Done(generation));
        return;
    }

    let mut builder = WalkBuilder::new(&roots[0]);
    for r in &roots[1..] {
        builder.add(r);
    }
    builder
        .hidden(!opts.include_hidden)
        .git_ignore(opts.respect_gitignore)
        .git_global(opts.respect_gitignore)
        .git_exclude(opts.respect_gitignore)
        .ignore(opts.respect_gitignore)
        .parents(opts.respect_gitignore)
        .follow_links(opts.follow_links);

    // Block mode uses our own splitter + matcher; line mode uses grep-searcher.
    let block_splitter = if query.granularity == Granularity::Block {
        Some(Arc::new(BlockSplitter::default()))
    } else {
        None
    };
    // Extractor registry: plain-text files search in place; binary formats
    // (pdf/xls/docx, when their features are enabled) decode to text first.
    let registry = Arc::new(default_registry());

    let walker = builder.build_parallel();
    walker.run(|| {
        let matcher = matcher.clone();
        let tx = tx.clone();
        let current_gen = current_gen.clone();
        let block_splitter = block_splitter.clone();
        let registry = registry.clone();
        Box::new(move |result| {
            // Cancelled? A newer search has started.
            if current_gen.load(Ordering::Relaxed) != generation {
                return WalkState::Quit;
            }
            let entry = match result {
                Ok(e) => e,
                Err(_) => return WalkState::Continue,
            };
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                return WalkState::Continue;
            }
            let path = entry.path().to_path_buf();
            match registry.resolve(&path).extract(&path) {
                // Plain text / logs: search the file in place.
                Ok(Some(Source::Raw)) => match &block_splitter {
                    Some(bs) => {
                        search_one_blocks(&matcher, &path, bs, generation, &current_gen, &tx)
                    }
                    None => search_one(&matcher, &path, generation, &current_gen, &tx),
                },
                // Decoded binary format: search the materialized text.
                Ok(Some(Source::Materialized(text))) => {
                    let cancelled = match &block_splitter {
                        Some(bs) => emit_block_matches(
                            &matcher,
                            &path,
                            bs,
                            &text,
                            generation,
                            &current_gen,
                            &tx,
                        ),
                        None => {
                            emit_line_matches(&matcher, &path, &text, generation, &current_gen, &tx)
                        }
                    };
                    if cancelled {
                        WalkState::Quit
                    } else {
                        WalkState::Continue
                    }
                }
                // Skip (unsupported / extraction failed).
                _ => WalkState::Continue,
            }
        })
    });

    let _ = tx.send(SearchEvent::Done(generation));
}

/// Search a single file's raw bytes with grep-searcher (line mode fast path).
fn search_one(
    matcher: &RegexMatcher,
    path: &Path,
    generation: u64,
    current_gen: &Arc<AtomicU64>,
    tx: &Sender<SearchEvent>,
) -> WalkState {
    let mut searcher: Searcher = SearcherBuilder::new().line_number(true).build();
    let sink = MatchSink {
        path,
        generation,
        current_gen,
        tx,
        stop: false,
    };
    let _ = searcher.search_path(matcher, path, sink);
    if current_gen.load(Ordering::Relaxed) != generation {
        WalkState::Quit
    } else {
        WalkState::Continue
    }
}

/// Search a single file in block mode: read it, split into timestamp blocks,
/// and emit one match per block that contains a matching line. Reused by the
/// watch module for block-mode re-scans.
pub(crate) fn search_one_blocks(
    matcher: &RegexMatcher,
    path: &Path,
    splitter: &BlockSplitter,
    generation: u64,
    current_gen: &Arc<AtomicU64>,
    tx: &Sender<SearchEvent>,
) -> WalkState {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return WalkState::Continue,
    };
    // Cheap binary guard: a NUL byte in the head means "not text".
    if bytes.iter().take(8192).any(|&b| b == 0) {
        return WalkState::Continue;
    }
    let text = String::from_utf8_lossy(&bytes);
    if emit_block_matches(matcher, path, splitter, &text, generation, current_gen, tx) {
        WalkState::Quit
    } else {
        WalkState::Continue
    }
}

/// Split in-memory `text` into blocks and emit one match per block containing a
/// match. Returns `true` if cancelled mid-way. Shared by on-disk block search
/// and by extractor-materialized text (pdf/xls/docx).
pub(crate) fn emit_block_matches(
    matcher: &RegexMatcher,
    path: &Path,
    splitter: &BlockSplitter,
    text: &str,
    generation: u64,
    current_gen: &Arc<AtomicU64>,
    tx: &Sender<SearchEvent>,
) -> bool {
    let mut cancelled = false;
    splitter.split(text, &mut |unit| {
        if current_gen.load(Ordering::Relaxed) != generation {
            cancelled = true;
            return false;
        }
        if let Some(matched_line) = first_matching_line(matcher, &unit) {
            // Char-safe cap so we never split a multi-byte codepoint.
            let mut display: String = unit.text.chars().take(BLOCK_DISPLAY_CAP).collect();
            if display.len() < unit.text.len() {
                display.push('…');
            }
            let m = Match {
                path: path.to_path_buf(),
                line_start: unit.line_start,
                line_end: unit.line_end,
                byte_offset: unit.byte_offset,
                text: display,
                matched_line,
            };
            if tx.send(SearchEvent::Match(generation, m)).is_err() {
                cancelled = true;
                return false;
            }
        }
        true
    });
    cancelled
}

/// Line-search in-memory `text` (extractor-materialized formats). Returns `true`
/// if cancelled. The on-disk line path uses grep-searcher instead (see
/// [`search_one`]).
pub(crate) fn emit_line_matches(
    matcher: &RegexMatcher,
    path: &Path,
    text: &str,
    generation: u64,
    current_gen: &Arc<AtomicU64>,
    tx: &Sender<SearchEvent>,
) -> bool {
    let mut byte_offset: u64 = 0;
    for (idx, line) in text.split_inclusive('\n').enumerate() {
        if current_gen.load(Ordering::Relaxed) != generation {
            return true;
        }
        let content = line.strip_suffix('\n').unwrap_or(line);
        let content = content.strip_suffix('\r').unwrap_or(content);
        if matcher.is_match(content.as_bytes()).unwrap_or(false) {
            let ln = idx as u64 + 1;
            let m = Match {
                path: path.to_path_buf(),
                line_start: ln,
                line_end: ln,
                byte_offset,
                text: content.to_string(),
                matched_line: ln,
            };
            if tx.send(SearchEvent::Match(generation, m)).is_err() {
                return true;
            }
        }
        byte_offset += line.len() as u64;
    }
    false
}

/// The first line within a block (by absolute line number) that matches.
fn first_matching_line(matcher: &RegexMatcher, unit: &Unit<'_>) -> Option<u64> {
    for (ln, line) in (unit.line_start..).zip(unit.text.split('\n')) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if matcher.is_match(line.as_bytes()).unwrap_or(false) {
            return Some(ln);
        }
    }
    None
}

/// grep-searcher sink that turns each matching line into a `SearchEvent::Match`
/// and applies backpressure by riding the bounded channel's blocking send.
struct MatchSink<'a> {
    path: &'a Path,
    generation: u64,
    current_gen: &'a Arc<AtomicU64>,
    tx: &'a Sender<SearchEvent>,
    stop: bool,
}

impl<'a> Sink for MatchSink<'a> {
    type Error = std::io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        if self.stop || self.current_gen.load(Ordering::Relaxed) != self.generation {
            return Ok(false);
        }
        let line_start = mat.line_number().unwrap_or(0);
        let byte_offset = mat.absolute_byte_offset();
        let text = String::from_utf8_lossy(mat.bytes())
            .trim_end_matches(['\r', '\n'])
            .to_string();
        let m = Match {
            path: self.path.to_path_buf(),
            line_start,
            line_end: line_start,
            byte_offset,
            text,
            matched_line: line_start,
        };
        // Blocking send == natural backpressure. If the receiver is gone
        // (UI closed / superseded), stop this file.
        match self.tx.send(SearchEvent::Match(self.generation, m)) {
            Ok(()) => Ok(true),
            Err(_) => {
                self.stop = true;
                Ok(false)
            }
        }
    }
}

/// Convenience for the CLI and tests: run a search on the current thread and
/// collect all matches. Uses a private generation, so cancellation is a no-op.
pub fn search_collect(roots: &[PathBuf], query: &Query, opts: ScanOptions) -> Vec<Match> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let gen = Arc::new(AtomicU64::new(1));
    search(roots, query, 1, gen, opts, tx);
    let mut out = Vec::new();
    for ev in rx.try_iter() {
        if let SearchEvent::Match(_, m) = ev {
            out.push(m);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpdir() -> PathBuf {
        // A unique-enough temp dir without external crates: PID + address.
        let base = std::env::temp_dir();
        let uniq = format!("insearch-test-{}-{:p}", std::process::id(), &base);
        let dir = base.join(uniq);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        p
    }

    #[test]
    fn finds_literal_matches_across_files() {
        let dir = tmpdir();
        write(&dir, "a.log", "hello world\nno match here\nhello again\n");
        write(&dir, "b.txt", "nothing\nHELLO upper\n");
        let q = Query::literal("hello");
        let mut hits = search_collect(std::slice::from_ref(&dir), &q, ScanOptions::default());
        hits.sort_by_key(|m| (m.path.clone(), m.line_start));
        // smart-case: "hello" (lowercase) matches case-insensitively -> 3 hits.
        assert_eq!(hits.len(), 3, "hits: {hits:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn smart_case_respects_uppercase_in_query() {
        let dir = tmpdir();
        write(&dir, "a.log", "hello\nHELLO\nHello\n");
        let q = Query {
            pattern: "HELLO".into(),
            is_regex: false,
            smart_case: true,
            granularity: crate::model::Granularity::Line,
        };
        let hits = search_collect(std::slice::from_ref(&dir), &q, ScanOptions::default());
        // Uppercase in query -> case-sensitive -> only the exact "HELLO".
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line_start, 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn block_mode_returns_one_hit_per_matching_block() {
        let dir = tmpdir();
        write(
            &dir,
            "app.log",
            "2026-01-01 00:00:01 start\n\
             2026-01-01 00:00:02 boom\n\
             \tcaused by: X\n\
             2026-01-01 00:00:03 done\n",
        );
        let q = Query {
            pattern: "caused by".into(),
            is_regex: false,
            smart_case: true,
            granularity: Granularity::Block,
        };
        let hits = search_collect(std::slice::from_ref(&dir), &q, ScanOptions::default());
        assert_eq!(hits.len(), 1, "hits: {hits:?}");
        // The match lives on the continuation line of the block spanning lines 2-3.
        assert_eq!(hits[0].line_start, 2);
        assert_eq!(hits[0].line_end, 3);
        assert_eq!(hits[0].matched_line, 3);
        assert!(hits[0].text.contains("boom") && hits[0].text.contains("caused by"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn invalid_regex_reports_error() {
        let q = Query {
            pattern: "(unclosed".into(),
            is_regex: true,
            smart_case: true,
            granularity: crate::model::Granularity::Line,
        };
        assert!(build_matcher(&q).is_err());
    }
}
