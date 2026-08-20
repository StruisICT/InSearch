//! Watch mode: monitor folders and re-scan files as they change.
//!
//! Shares the engine's extract→split→match core with live search; only the
//! *trigger* differs (filesystem events instead of a query change). A debounced
//! `notify` watcher feeds a worker thread that, per changed file:
//!
//!   * **line mode** — tails the file: reads only the bytes appended since the
//!     last event (tracked in an offset map), so a growing log streams new
//!     matches without re-reading the whole file. A shrunk file (truncation /
//!     log rotation) resets the offset and re-scans.
//!   * **block mode** — re-reads the whole file (a [`SearchEvent::Clear`] first
//!     drops its stale matches). Block-mode tailing of an *open* trailing block
//!     is a known limitation, noted for a later pass.
//!
//! Cancellation rides the same generation counter as live search: bump it and
//! the worker exits at the next event; dropping the returned [`WatchHandle`]
//! stops the watcher.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Sender;
use grep_matcher::Matcher as _;
use grep_regex::RegexMatcher;
use notify_debouncer_full::notify::event::ModifyKind;
use notify_debouncer_full::notify::{self, EventKind, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{
    new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer, RecommendedCache,
};

use crate::model::{Granularity, Match, Query, SearchEvent};
use crate::scan::{build_matcher, search_one_blocks};
use crate::split::BlockSplitter;

/// Debounce window for coalescing bursts of filesystem events.
const DEBOUNCE: Duration = Duration::from_millis(500);
/// Bytes to sniff for a NUL when deciding a file is binary.
const BINARY_SNIFF: usize = 8192;

/// Keeps a watch alive. Drop it to stop watching and let the worker exit.
pub struct WatchHandle {
    // Dropping the debouncer stops the OS watcher and disconnects the event
    // channel, which ends the worker thread.
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
}

/// Start watching `roots` for changes, streaming incremental match updates on
/// `tx` tagged with `generation`. The caller is expected to have already run a
/// full initial scan on the same channel/generation to populate results.
pub fn start(
    roots: &[PathBuf],
    query: &Query,
    generation: u64,
    current_gen: Arc<AtomicU64>,
    tx: Sender<SearchEvent>,
) -> notify::Result<WatchHandle> {
    // The debouncer's event handler is implemented for std's mpsc Sender (the
    // crossbeam impl is feature-gated off), so use std mpsc for the event feed.
    let (ev_tx, ev_rx) = std::sync::mpsc::channel::<DebounceEventResult>();
    let mut debouncer = new_debouncer(DEBOUNCE, None, ev_tx)?;

    for root in roots {
        // Watch directories recursively; watching a single file also works but
        // watching its parent dir is what notify does under the hood anyway.
        let target: &Path = if root.is_file() {
            root.parent().unwrap_or(root)
        } else {
            root
        };
        debouncer.watch(target, RecursiveMode::Recursive)?;
    }

    let query = query.clone();
    std::thread::spawn(move || {
        let mut tailer = match Tailer::new(query, generation, current_gen, tx) {
            Some(t) => t,
            None => return, // invalid regex — the initial scan already reported it
        };
        for result in ev_rx.iter() {
            if tailer.cancelled() {
                break;
            }
            if let Ok(events) = result {
                for ev in &events {
                    tailer.handle(ev);
                    if tailer.cancelled() {
                        break;
                    }
                }
            }
        }
    });

    Ok(WatchHandle {
        _debouncer: debouncer,
    })
}

/// Per-file tailing state: how far we've read and how many lines precede it.
struct FileState {
    offset: u64,
    line_base: u64,
}

/// Processes debounced events into incremental match updates.
struct Tailer {
    matcher: RegexMatcher,
    block: Option<BlockSplitter>,
    generation: u64,
    current_gen: Arc<AtomicU64>,
    tx: Sender<SearchEvent>,
    offsets: HashMap<PathBuf, FileState>,
}

impl Tailer {
    fn new(
        query: Query,
        generation: u64,
        current_gen: Arc<AtomicU64>,
        tx: Sender<SearchEvent>,
    ) -> Option<Self> {
        let matcher = build_matcher(&query).ok()?;
        let block = if query.granularity == Granularity::Block {
            Some(BlockSplitter::default())
        } else {
            None
        };
        Some(Tailer {
            matcher,
            block,
            generation,
            current_gen,
            tx,
            offsets: HashMap::new(),
        })
    }

    fn cancelled(&self) -> bool {
        self.current_gen.load(Ordering::Relaxed) != self.generation
    }

    fn send(&self, ev: SearchEvent) -> bool {
        self.tx.send(ev).is_ok()
    }

    fn handle(&mut self, ev: &DebouncedEvent) {
        match ev.kind {
            EventKind::Remove(_) => {
                for p in &ev.paths {
                    self.remove(p);
                }
            }
            EventKind::Create(_)
            | EventKind::Modify(ModifyKind::Data(_))
            | EventKind::Modify(ModifyKind::Any)
            | EventKind::Modify(ModifyKind::Other) => {
                for p in &ev.paths {
                    self.rescan(p);
                }
            }
            // Renames report both old and new paths; rescan those that exist,
            // evict those that don't.
            EventKind::Modify(ModifyKind::Name(_)) => {
                for p in &ev.paths {
                    if p.is_file() {
                        self.rescan(p);
                    } else {
                        self.remove(p);
                    }
                }
            }
            // Metadata-only changes and access events don't affect content.
            _ => {}
        }
    }

    fn remove(&mut self, path: &Path) {
        self.offsets.remove(path);
        self.send(SearchEvent::Clear(self.generation, path.to_path_buf()));
    }

    fn rescan(&mut self, path: &Path) {
        if !path.is_file() {
            return;
        }
        match self.block {
            Some(_) => self.rescan_block(path),
            None => self.tail_lines(path),
        }
    }

    /// Block mode: clear the file's prior matches and re-run block search.
    fn rescan_block(&mut self, path: &Path) {
        if !self.send(SearchEvent::Clear(self.generation, path.to_path_buf())) {
            return;
        }
        if let Some(bs) = &self.block {
            let _ = search_one_blocks(
                &self.matcher,
                path,
                bs,
                self.generation,
                &self.current_gen,
                &self.tx,
            );
        }
    }

    /// Line mode: read only the bytes appended since last time and emit matches
    /// for the newly completed lines (absolute line numbers preserved).
    fn tail_lines(&mut self, path: &Path) {
        let size = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(_) => return,
        };

        // First time we see this file, clear whatever the initial scan produced
        // for it and tail from the start (re-emitting its current matches once).
        let first_seen = !self.offsets.contains_key(path);
        if first_seen {
            if !self.send(SearchEvent::Clear(self.generation, path.to_path_buf())) {
                return;
            }
            self.offsets.insert(
                path.to_path_buf(),
                FileState {
                    offset: 0,
                    line_base: 0,
                },
            );
        }

        // Copy state out so we don't hold a borrow across `self.send`.
        let (mut start, mut line_base) = {
            let st = self.offsets.get(path).expect("state inserted above");
            (st.offset, st.line_base)
        };

        // Truncation / rotation: file shrank below where we'd read to.
        if size < start {
            start = 0;
            line_base = 0;
            if !self.send(SearchEvent::Clear(self.generation, path.to_path_buf())) {
                return;
            }
        }

        let mut file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return,
        };
        if file.seek(SeekFrom::Start(start)).is_err() {
            return;
        }
        let mut buf = Vec::new();
        if file.read_to_end(&mut buf).is_err() {
            return;
        }
        // Binary guard on the very first read.
        if start == 0 && buf.iter().take(BINARY_SNIFF).any(|&b| b == 0) {
            return;
        }

        // Only consume up to the last newline; a trailing partial line may still
        // be growing, so leave it for the next event.
        let complete_end = match buf.iter().rposition(|&b| b == b'\n') {
            Some(i) => i + 1,
            None => return, // no complete new line yet
        };
        let text = String::from_utf8_lossy(&buf[..complete_end]);

        let base_line = line_base;
        let mut local_line: u64 = 0;
        let mut byte_in_chunk: u64 = 0;
        for line in text.split_inclusive('\n') {
            local_line += 1;
            let content = line.strip_suffix('\n').unwrap_or(line);
            let content = content.strip_suffix('\r').unwrap_or(content);
            if self.matcher.is_match(content.as_bytes()).unwrap_or(false) {
                let m = Match {
                    path: path.to_path_buf(),
                    line_start: base_line + local_line,
                    line_end: base_line + local_line,
                    byte_offset: start + byte_in_chunk,
                    text: content.to_string(),
                    matched_line: base_line + local_line,
                };
                if !self.send(SearchEvent::Match(self.generation, m)) {
                    return;
                }
            }
            byte_in_chunk += line.len() as u64;
        }

        if let Some(st) = self.offsets.get_mut(path) {
            st.offset = start + complete_end as u64;
            st.line_base = base_line + local_line;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Granularity;
    use std::io::Write;

    fn tmp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fetchy-watch-{}-{}",
            std::process::id(),
            name.replace(['/', '\\', '.'], "_")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn append(path: &Path, s: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(s.as_bytes()).unwrap();
    }

    fn line_query() -> Query {
        Query {
            pattern: "ERROR".into(),
            is_regex: false,
            smart_case: true,
            granularity: Granularity::Line,
        }
    }

    fn drain(rx: &crossbeam_channel::Receiver<SearchEvent>) -> Vec<SearchEvent> {
        rx.try_iter().collect()
    }

    #[test]
    fn tailing_reports_only_newly_appended_lines() {
        let path = tmp_file("t.log");
        let _ = std::fs::remove_file(&path);
        append(&path, "2026 INFO a\n2026 ERROR b\n");

        let (tx, rx) = crossbeam_channel::unbounded();
        let current_gen = Arc::new(AtomicU64::new(7));
        let mut tailer = Tailer::new(line_query(), 7, current_gen, tx).unwrap();

        // First pass: clears prior (initial-scan) matches, then reports line 2.
        tailer.tail_lines(&path);
        let evs = drain(&rx);
        assert!(matches!(evs[0], SearchEvent::Clear(7, _)));
        let matches: Vec<_> = evs
            .iter()
            .filter_map(|e| match e {
                SearchEvent::Match(_, m) => Some(m.line_start),
                _ => None,
            })
            .collect();
        assert_eq!(matches, vec![2]);

        // Append one matching complete line + a partial (no newline yet).
        append(&path, "2026 ERROR c\npartial-still-writing");
        tailer.tail_lines(&path);
        let evs = drain(&rx);
        // No Clear this time; only the new line 3 (partial line withheld).
        assert!(!evs.iter().any(|e| matches!(e, SearchEvent::Clear(..))));
        let matches: Vec<_> = evs
            .iter()
            .filter_map(|e| match e {
                SearchEvent::Match(_, m) => Some(m.line_start),
                _ => None,
            })
            .collect();
        assert_eq!(matches, vec![3]);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn truncation_resets_and_reclears() {
        let path = tmp_file("rot.log");
        let _ = std::fs::remove_file(&path);
        append(&path, "2026 ERROR one\n2026 ERROR two\n");

        let (tx, rx) = crossbeam_channel::unbounded();
        let current_gen = Arc::new(AtomicU64::new(1));
        let mut tailer = Tailer::new(line_query(), 1, current_gen, tx).unwrap();

        tailer.tail_lines(&path);
        let first = drain(&rx);
        let first_matches = first
            .iter()
            .filter(|e| matches!(e, SearchEvent::Match(..)))
            .count();
        assert_eq!(first_matches, 2);

        // Rotate: replace with a shorter file.
        std::fs::write(&path, "2026 INFO fresh\n").unwrap();
        tailer.tail_lines(&path);
        let evs = drain(&rx);
        // Truncation forces a Clear; the fresh line has no match.
        assert!(evs.iter().any(|e| matches!(e, SearchEvent::Clear(..))));
        assert!(!evs.iter().any(|e| matches!(e, SearchEvent::Match(..))));

        std::fs::remove_file(&path).ok();
    }
}
