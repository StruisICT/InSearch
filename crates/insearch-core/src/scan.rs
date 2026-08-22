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
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crossbeam_channel::Sender;
use grep_matcher::Matcher as _;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkMatch};
use ignore::{DirEntry, WalkBuilder, WalkState};

use crate::extract::{default_registry, Source};
use crate::model::{CaseMode, Granularity, Match, MatchMode, Query, SearchEvent};
use crate::split::{strip_eol, BlockSplitter, Unit, UnitSplitter};

/// Cap block text length (in characters) held per match for display.
const BLOCK_DISPLAY_CAP: usize = 2000;

/// Block mode reads whole files into memory (the line path streams via
/// grep-searcher instead). Skip files above this to bound memory on huge logs.
const MAX_BLOCK_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Options controlling the walk. Sensible defaults for a document/log searcher:
/// see everything (don't honour `.gitignore`, do descend hidden dirs).
#[derive(Clone, Debug)]
pub struct ScanOptions {
    pub respect_gitignore: bool,
    pub include_hidden: bool,
    pub follow_links: bool,
    /// Restrict which files are searched (name / extension / size / age).
    pub filter: FileFilter,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions {
            respect_gitignore: false,
            include_hidden: true,
            follow_links: false,
            filter: FileFilter::default(),
        }
    }
}

/// Restricts which files a search visits. Empty fields impose no restriction, so
/// `FileFilter::default()` matches everything.
#[derive(Clone, Debug, Default)]
pub struct FileFilter {
    /// Match the file *name* against this pattern (glob by default, or a regex
    /// if `name_is_regex`). Empty = no name filter. Case-insensitive.
    pub name_pattern: String,
    pub name_is_regex: bool,
    /// Lowercase extensions to include (without the dot). Empty = all.
    pub include_exts: Vec<String>,
    /// Lowercase extensions to exclude (takes precedence over include).
    pub exclude_exts: Vec<String>,
    /// Inclusive size bounds, in bytes.
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    /// Only files modified within this many days (None = any age). A relative
    /// window; combined with `modified_after` by taking the later bound.
    pub modified_within_days: Option<u64>,
    /// Absolute lower bound: only files modified at/after this instant.
    pub modified_after: Option<SystemTime>,
    /// Absolute upper bound: only files modified at/before this instant.
    pub modified_before: Option<SystemTime>,
}

/// Translate a shell-style glob (`*`, `?`) into an anchored, case-insensitive
/// regex source. All other characters are treated literally.
fn glob_to_regex(glob: &str) -> String {
    let mut re = String::from("(?i)^");
    for ch in glob.chars() {
        match ch {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
    }
    re.push('$');
    re
}

/// A [`FileFilter`] with its name pattern compiled and its age bound resolved to
/// an absolute time, ready to test candidate files.
struct CompiledFilter {
    name: Option<regex::Regex>,
    include_exts: Vec<String>,
    exclude_exts: Vec<String>,
    min_size: Option<u64>,
    max_size: Option<u64>,
    modified_after: Option<SystemTime>,
    modified_before: Option<SystemTime>,
}

impl CompiledFilter {
    fn new(f: &FileFilter, now: SystemTime) -> Self {
        let name = if f.name_pattern.is_empty() {
            None
        } else {
            let src = if f.name_is_regex {
                format!("(?i){}", f.name_pattern)
            } else {
                glob_to_regex(&f.name_pattern)
            };
            regex::Regex::new(&src).ok()
        };
        // The relative "within N days" window and the absolute lower bound are
        // both lower bounds on modification time; the effective floor is the
        // later (more restrictive) of the two.
        let days_after = f
            .modified_within_days
            .map(|d| now - Duration::from_secs(d.saturating_mul(86_400)));
        let modified_after = match (days_after, f.modified_after) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        CompiledFilter {
            name,
            include_exts: f.include_exts.clone(),
            exclude_exts: f.exclude_exts.clone(),
            min_size: f.min_size,
            max_size: f.max_size,
            modified_after,
            modified_before: f.modified_before,
        }
    }

    /// Whether any check is active — lets the walker skip work when unfiltered.
    fn is_active(&self) -> bool {
        self.name.is_some()
            || !self.include_exts.is_empty()
            || !self.exclude_exts.is_empty()
            || self.min_size.is_some()
            || self.max_size.is_some()
            || self.modified_after.is_some()
            || self.modified_before.is_some()
    }

    fn needs_metadata(&self) -> bool {
        self.min_size.is_some()
            || self.max_size.is_some()
            || self.modified_after.is_some()
            || self.modified_before.is_some()
    }

    /// Does this file pass the filter?
    fn accepts(&self, entry: &DirEntry) -> bool {
        let path = entry.path();

        // Extension include/exclude (cheap, from the path).
        if !self.include_exts.is_empty() || !self.exclude_exts.is_empty() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .unwrap_or_default();
            if self.exclude_exts.contains(&ext) {
                return false;
            }
            if !self.include_exts.is_empty() && !self.include_exts.contains(&ext) {
                return false;
            }
        }

        // Name pattern.
        if let Some(re) = &self.name {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if !re.is_match(name) {
                return false;
            }
        }

        // Size / age (needs a stat, so only when those bounds are set).
        if self.needs_metadata() {
            let md = match entry.metadata() {
                Ok(m) => m,
                Err(_) => return false,
            };
            let size = md.len();
            if self.min_size.is_some_and(|min| size < min) {
                return false;
            }
            if self.max_size.is_some_and(|max| size > max) {
                return false;
            }
            if self.modified_after.is_some() || self.modified_before.is_some() {
                let m = match md.modified() {
                    Ok(m) => m,
                    Err(_) => return false,
                };
                if self.modified_after.is_some_and(|after| m < after) {
                    return false;
                }
                if self.modified_before.is_some_and(|before| m > before) {
                    return false;
                }
            }
        }

        true
    }
}

/// A query compiled into concrete matchers: a unit matches when *every*
/// `required` matcher matches and *no* `excluded` matcher does. `primary` (a
/// superset — the first required matcher) is used to drive grep-searcher's line
/// scan and to locate the matched line within a block.
pub(crate) struct CompiledQuery {
    required: Vec<RegexMatcher>,
    excluded: Vec<RegexMatcher>,
    primary: RegexMatcher,
}

impl CompiledQuery {
    /// Does `bytes` satisfy all required matchers and no excluded matcher?
    pub(crate) fn unit_matches(&self, bytes: &[u8]) -> bool {
        self.required
            .iter()
            .all(|m| m.is_match(bytes).unwrap_or(false))
            && !self
                .excluded
                .iter()
                .any(|m| m.is_match(bytes).unwrap_or(false))
    }
}

/// Wrap a regex source in word boundaries when whole-word matching is on.
fn wrap_word(src: String, whole_word: bool) -> String {
    if whole_word {
        format!(r"\b(?:{src})\b")
    } else {
        src
    }
}

/// Regex source for a single literal term.
fn literal_term(term: &str, whole_word: bool) -> String {
    wrap_word(regex::escape(term), whole_word)
}

/// Build one matcher from an already-regex source, honouring the case policy.
fn build_one(src: &str, case: CaseMode) -> Result<RegexMatcher, String> {
    let mut b = RegexMatcherBuilder::new();
    match case {
        CaseMode::Smart => {
            b.case_smart(true);
        }
        CaseMode::Sensitive => {
            b.case_insensitive(false);
        }
        CaseMode::Insensitive => {
            b.case_insensitive(true);
        }
    }
    b.build(src).map_err(|e| e.to_string())
}

/// A compiled regex matching any of the query's *positive* terms, for UI match
/// highlighting. Returns `None` for an empty or invalid query.
pub fn highlight_regex(query: &Query) -> Option<regex::Regex> {
    let ww = query.whole_word;
    let mut alts: Vec<String> = Vec::new();
    match query.mode {
        MatchMode::Substring => alts.push(literal_term(&query.pattern, ww)),
        MatchMode::Regex => alts.push(wrap_word(query.pattern.clone(), ww)),
        MatchMode::AllWords | MatchMode::AnyWords => {
            for w in query.pattern.split_whitespace() {
                alts.push(literal_term(w, ww));
            }
        }
    }
    alts.retain(|s| !s.is_empty());
    if alts.is_empty() {
        return None;
    }
    let combined = alts.join("|");
    let insensitive = match query.case {
        CaseMode::Insensitive => true,
        CaseMode::Sensitive => false,
        CaseMode::Smart => !query.pattern.chars().any(|c| c.is_uppercase()),
    };
    let src = if insensitive {
        format!("(?i){combined}")
    } else {
        combined
    };
    regex::Regex::new(&src).ok()
}

/// Compile a [`Query`] into a [`CompiledQuery`].
pub(crate) fn build_compiled(query: &Query) -> Result<CompiledQuery, String> {
    let ww = query.whole_word;
    let mut required_src: Vec<String> = Vec::new();
    match query.mode {
        MatchMode::Substring => required_src.push(literal_term(&query.pattern, ww)),
        MatchMode::Regex => required_src.push(wrap_word(query.pattern.clone(), ww)),
        MatchMode::AllWords => {
            // Each word is a separate required matcher (order-independent AND).
            for w in query.pattern.split_whitespace() {
                required_src.push(literal_term(w, ww));
            }
        }
        MatchMode::AnyWords => {
            let alts: Vec<String> = query
                .pattern
                .split_whitespace()
                .map(|w| literal_term(w, ww))
                .collect();
            if !alts.is_empty() {
                required_src.push(alts.join("|"));
            }
        }
    }
    if required_src.is_empty() {
        return Err("empty query".into());
    }

    let required: Vec<RegexMatcher> = required_src
        .iter()
        .map(|s| build_one(s, query.case))
        .collect::<Result<_, _>>()?;
    let excluded: Vec<RegexMatcher> = query
        .exclude
        .split_whitespace()
        .map(|w| build_one(&literal_term(w, ww), query.case))
        .collect::<Result<_, _>>()?;
    let primary = required[0].clone();
    Ok(CompiledQuery {
        required,
        excluded,
        primary,
    })
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
    scanned: Arc<AtomicUsize>,
) {
    let compiled = match build_compiled(query) {
        Ok(c) => Arc::new(c),
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
    // File filter (name / extension / size / age), compiled once per search.
    let now = SystemTime::now();
    let filter = Arc::new(CompiledFilter::new(&opts.filter, now));
    let filter_active = filter.is_active();

    let walker = builder.build_parallel();
    walker.run(|| {
        let compiled = compiled.clone();
        let tx = tx.clone();
        let current_gen = current_gen.clone();
        let block_splitter = block_splitter.clone();
        let registry = registry.clone();
        let filter = filter.clone();
        let scanned = scanned.clone();
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
            scanned.fetch_add(1, Ordering::Relaxed);
            if filter_active && !filter.accepts(&entry) {
                return WalkState::Continue;
            }
            let path = entry.path().to_path_buf();
            match registry.resolve(&path).extract(&path) {
                // Plain text / logs: search the file in place.
                Ok(Some(Source::Raw)) => match &block_splitter {
                    Some(bs) => {
                        search_file_blocks(&compiled, &path, bs, generation, &current_gen, &tx)
                    }
                    None => search_file_lines(&compiled, &path, generation, &current_gen, &tx),
                },
                // Decoded binary format: search the materialized text.
                Ok(Some(Source::Materialized(text))) => {
                    let cancelled = match &block_splitter {
                        Some(bs) => search_text_blocks(
                            &compiled,
                            &path,
                            bs,
                            &text,
                            generation,
                            &current_gen,
                            &tx,
                        ),
                        None => search_text_lines(
                            &compiled,
                            &path,
                            &text,
                            generation,
                            &current_gen,
                            &tx,
                        ),
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
/// grep-searcher scans for the `primary` matcher; the sink then verifies the
/// full compiled query (all required, no excluded) on each candidate line.
fn search_file_lines(
    compiled: &CompiledQuery,
    path: &Path,
    generation: u64,
    current_gen: &Arc<AtomicU64>,
    tx: &Sender<SearchEvent>,
) -> WalkState {
    let mut searcher: Searcher = SearcherBuilder::new().line_number(true).build();
    let sink = MatchSink {
        compiled,
        path,
        generation,
        current_gen,
        tx,
        stop: false,
    };
    let _ = searcher.search_path(&compiled.primary, path, sink);
    if current_gen.load(Ordering::Relaxed) != generation {
        WalkState::Quit
    } else {
        WalkState::Continue
    }
}

/// Search a single file in block mode: read it, split into timestamp blocks,
/// and emit one match per block that contains a matching line. Reused by the
/// watch module for block-mode re-scans.
pub(crate) fn search_file_blocks(
    compiled: &CompiledQuery,
    path: &Path,
    splitter: &BlockSplitter,
    generation: u64,
    current_gen: &Arc<AtomicU64>,
    tx: &Sender<SearchEvent>,
) -> WalkState {
    // Bound memory: skip files too large to read whole into RAM in block mode.
    if std::fs::metadata(path)
        .map(|m| m.len() > MAX_BLOCK_FILE_BYTES)
        .unwrap_or(false)
    {
        return WalkState::Continue;
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return WalkState::Continue,
    };
    // Cheap binary guard: a NUL byte in the head means "not text".
    if bytes.iter().take(8192).any(|&b| b == 0) {
        return WalkState::Continue;
    }
    let text = String::from_utf8_lossy(&bytes);
    if search_text_blocks(compiled, path, splitter, &text, generation, current_gen, tx) {
        WalkState::Quit
    } else {
        WalkState::Continue
    }
}

/// Split in-memory `text` into blocks and emit one match per block containing a
/// match. Returns `true` if cancelled mid-way. Shared by on-disk block search
/// and by extractor-materialized text (pdf/xls/docx).
pub(crate) fn search_text_blocks(
    compiled: &CompiledQuery,
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
        // A block matches when the whole block satisfies the query (all required
        // terms present somewhere in it, no excluded term); the reported line is
        // the first one containing the primary term.
        if compiled.unit_matches(unit.text.as_bytes()) {
            let matched_line =
                first_matching_line(&compiled.primary, &unit).unwrap_or(unit.line_start);
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
/// [`search_file_lines`]).
pub(crate) fn search_text_lines(
    compiled: &CompiledQuery,
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
        let content = strip_eol(line);
        if compiled.unit_matches(content.as_bytes()) {
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

/// The first line within a block (by absolute line number) that matches. The
/// block text is already free of its trailing terminator, so `split('\n')` won't
/// yield a dangling empty segment; `strip_eol` handles any interior `\r`.
fn first_matching_line(matcher: &RegexMatcher, unit: &Unit<'_>) -> Option<u64> {
    for (ln, line) in (unit.line_start..).zip(unit.text.split('\n')) {
        let line = strip_eol(line);
        if matcher.is_match(line.as_bytes()).unwrap_or(false) {
            return Some(ln);
        }
    }
    None
}

/// grep-searcher sink that turns each matching line into a `SearchEvent::Match`
/// and applies backpressure by riding the bounded channel's blocking send.
struct MatchSink<'a> {
    compiled: &'a CompiledQuery,
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
        // grep-searcher matched the primary term; confirm the full query (all
        // required, no excluded) before emitting.
        if !self.compiled.unit_matches(mat.bytes()) {
            return Ok(true);
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
    let scanned = Arc::new(AtomicUsize::new(0));
    search(roots, query, 1, gen, opts, tx, scanned);
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
            mode: MatchMode::Substring,
            case: CaseMode::Smart,
            granularity: crate::model::Granularity::Line,
            ..Query::literal("")
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
            granularity: Granularity::Block,
            ..Query::literal("caused by")
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
    fn glob_translation_matches_expected_names() {
        let re = regex::Regex::new(&glob_to_regex("*.log")).unwrap();
        assert!(re.is_match("app.log"));
        assert!(re.is_match("APP.LOG")); // case-insensitive
        assert!(!re.is_match("app.txt"));
        let re2 = regex::Regex::new(&glob_to_regex("data?.json")).unwrap();
        assert!(re2.is_match("data1.json"));
        assert!(!re2.is_match("data12.json")); // ? is exactly one char
    }

    #[test]
    fn file_filter_restricts_by_extension() {
        let dir = tmpdir();
        write(&dir, "a.log", "hello ERROR\n");
        write(&dir, "b.txt", "hello ERROR\n");
        write(&dir, "notes.md", "ERROR here\n");
        let opts = ScanOptions {
            filter: FileFilter {
                include_exts: vec!["log".into(), "txt".into()],
                ..FileFilter::default()
            },
            ..ScanOptions::default()
        };
        let q = Query::literal("ERROR");
        let hits = search_collect(std::slice::from_ref(&dir), &q, opts);
        let names: Vec<String> = hits
            .iter()
            .filter_map(|m| m.path.file_name().map(|s| s.to_string_lossy().into_owned()))
            .collect();
        assert!(names.iter().any(|n| n == "a.log"));
        assert!(names.iter().any(|n| n == "b.txt"));
        assert!(!names.iter().any(|n| n == "notes.md"), "names: {names:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_filter_by_absolute_date() {
        let dir = tmpdir();
        write(&dir, "now.log", "ERROR fresh\n"); // mtime ≈ now
        let q = Query::literal("ERROR");
        let count = |f: FileFilter| {
            let opts = ScanOptions {
                filter: f,
                ..ScanOptions::default()
            };
            search_collect(std::slice::from_ref(&dir), &q, opts).len()
        };

        // 2000-01-01 UTC — comfortably before the just-written file.
        let past = SystemTime::UNIX_EPOCH + Duration::from_secs(946_684_800);
        let future = SystemTime::now() + Duration::from_secs(86_400 * 3650);

        // "before a past date" excludes it; "after a past date" includes it.
        assert_eq!(
            count(FileFilter {
                modified_before: Some(past),
                ..FileFilter::default()
            }),
            0
        );
        assert_eq!(
            count(FileFilter {
                modified_after: Some(past),
                ..FileFilter::default()
            }),
            1
        );
        // "after a far-future date" excludes it.
        assert_eq!(
            count(FileFilter {
                modified_after: Some(future),
                ..FileFilter::default()
            }),
            0
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_filter_by_name_glob() {
        let dir = tmpdir();
        write(&dir, "server.log", "ERROR boot\n");
        write(&dir, "client.log", "ERROR boot\n");
        let opts = ScanOptions {
            filter: FileFilter {
                name_pattern: "server.*".into(),
                ..FileFilter::default()
            },
            ..ScanOptions::default()
        };
        let q = Query::literal("ERROR");
        let hits = search_collect(std::slice::from_ref(&dir), &q, opts);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.ends_with("server.log"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn all_words_requires_every_term_on_line() {
        let dir = tmpdir();
        write(
            &dir,
            "a.log",
            "alpha and beta together\nonly alpha here\nbeta only\n",
        );
        let q = Query {
            mode: MatchMode::AllWords,
            ..Query::literal("alpha beta")
        };
        let hits = search_collect(std::slice::from_ref(&dir), &q, ScanOptions::default());
        assert_eq!(hits.len(), 1, "hits: {hits:?}");
        assert_eq!(hits[0].line_start, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn any_words_matches_either_term() {
        let dir = tmpdir();
        write(&dir, "a.log", "has alpha\nhas beta\nhas gamma\n");
        let q = Query {
            mode: MatchMode::AnyWords,
            ..Query::literal("alpha beta")
        };
        let hits = search_collect(std::slice::from_ref(&dir), &q, ScanOptions::default());
        assert_eq!(hits.len(), 2, "hits: {hits:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exclude_term_rejects_matching_lines() {
        let dir = tmpdir();
        write(&dir, "a.log", "error in module\nerror but ignored\n");
        let q = Query {
            exclude: "ignored".into(),
            ..Query::literal("error")
        };
        let hits = search_collect(std::slice::from_ref(&dir), &q, ScanOptions::default());
        assert_eq!(hits.len(), 1, "hits: {hits:?}");
        assert_eq!(hits[0].line_start, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn whole_word_does_not_match_substrings() {
        let dir = tmpdir();
        write(&dir, "a.log", "cat\ncategory\nscattered\n");
        let q = Query {
            whole_word: true,
            ..Query::literal("cat")
        };
        let hits = search_collect(std::slice::from_ref(&dir), &q, ScanOptions::default());
        assert_eq!(hits.len(), 1, "hits: {hits:?}");
        assert_eq!(hits[0].line_start, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn invalid_regex_reports_error() {
        let q = Query {
            mode: MatchMode::Regex,
            ..Query::literal("(unclosed")
        };
        assert!(build_compiled(&q).is_err());
    }
}
