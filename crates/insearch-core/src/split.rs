//! Layer 2 of the extractor design: turning text into searchable *units*.
//!
//! A "unit" is a line (line mode) or a timestamp-to-timestamp block (block
//! mode). Splitting is format-agnostic: the same [`LineSplitter`] /
//! [`BlockSplitter`] run over a plain log file and over text extracted from a
//! PDF, so line-vs-block is a property of the view, not the file format.

/// A slice of a file's text plus where it came from.
#[derive(Clone, Copy, Debug)]
pub struct Unit<'a> {
    /// The unit's text (a single line, or a whole block).
    pub text: &'a str,
    /// 1-based line number of the unit's first line.
    pub line_start: u64,
    /// 1-based line number of the unit's last line (== `line_start` for a line).
    pub line_end: u64,
    /// Byte offset of the unit's start within the text.
    pub byte_offset: u64,
}

/// Strip a single trailing line terminator (`\n` or `\r\n`) from a line,
/// returning the content without reallocating.
pub(crate) fn strip_eol(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

/// Splits text into units. Implementations must be cheap to share across
/// threads.
pub trait UnitSplitter: Send + Sync {
    /// Invoke `sink` once per unit, in order. Returning `false` from `sink`
    /// stops iteration early.
    fn split<'a>(&self, text: &'a str, sink: &mut dyn FnMut(Unit<'a>) -> bool);
}

/// One unit per line. Line terminators are not included in `Unit::text`.
#[derive(Clone, Copy, Debug, Default)]
pub struct LineSplitter;

impl UnitSplitter for LineSplitter {
    fn split<'a>(&self, text: &'a str, sink: &mut dyn FnMut(Unit<'a>) -> bool) {
        // Track line number and byte offset together as we walk the lines.
        let mut offset: u64 = 0;
        for (idx, line) in text.split_inclusive('\n').enumerate() {
            let line_no = idx as u64 + 1;
            // Drop the trailing terminator for the unit text; `offset` below
            // still advances by the full line length to keep byte accounting.
            let keep = sink(Unit {
                text: strip_eol(line),
                line_start: line_no,
                line_end: line_no,
                byte_offset: offset,
            });
            offset += line.len() as u64;
            if !keep {
                return;
            }
        }
    }
}

/// Default cap on the number of lines in a single block. A run of lines with no
/// timestamp (e.g. a giant JSON dump) is force-split at this many lines so one
/// unit never balloons.
pub const DEFAULT_MAX_BLOCK_LINES: usize = 2000;

/// Ordered timestamp patterns (first match wins). A line whose start matches any
/// of these begins a new block. Users can override the set via
/// [`BlockSplitter::with_patterns`].
pub const DEFAULT_TS_PATTERNS: &[&str] = &[
    r"^\[?\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}", // ISO-8601 (optionally bracketed)
    r"^[A-Z][a-z]{2}\s+\d{1,2}\s+\d{2}:\d{2}:\d{2}", // syslog: "Aug 20 08:00:01"
    r"^\d{2}/\d{2}/\d{4}\s+\d{2}:\d{2}:\d{2}",     // 08/20/2026 08:00:01
    r"^\[?\d{10,13}\]?\b",                         // epoch seconds/millis
];

/// Groups lines into timestamp-to-timestamp blocks.
///
/// A line that starts with a recognised timestamp opens a new block; following
/// lines without a timestamp (stack traces, pretty-printed JSON, …) belong to
/// it. Leading lines before the first timestamp form an orphan block. Blocks are
/// capped at `max_lines` so an untimestamped run can't produce one huge unit.
pub struct BlockSplitter {
    ts: regex::Regex,
    max_lines: usize,
}

impl BlockSplitter {
    /// Build with a custom, ordered set of anchored timestamp patterns.
    pub fn with_patterns(patterns: &[&str], max_lines: usize) -> Result<Self, regex::Error> {
        let combined = patterns
            .iter()
            .map(|p| format!("(?:{p})"))
            .collect::<Vec<_>>()
            .join("|");
        Ok(BlockSplitter {
            ts: regex::Regex::new(&combined)?,
            max_lines: max_lines.max(1),
        })
    }

    /// Whether a line begins with a recognised timestamp.
    pub fn is_timestamp_line(&self, line: &str) -> bool {
        self.ts.is_match(line)
    }
}

impl Default for BlockSplitter {
    fn default() -> Self {
        // The default patterns are known-valid, so this cannot fail.
        BlockSplitter::with_patterns(DEFAULT_TS_PATTERNS, DEFAULT_MAX_BLOCK_LINES)
            .expect("default timestamp patterns compile")
    }
}

impl UnitSplitter for BlockSplitter {
    // `cur_line` is not a plain index: it feeds `cur_line - 1` for block ends and
    // is read after the loop for the trailing block, so `enumerate` doesn't fit.
    #[allow(clippy::explicit_counter_loop)]
    fn split<'a>(&self, text: &'a str, sink: &mut dyn FnMut(Unit<'a>) -> bool) {
        let mut block_start_byte: usize = 0;
        let mut block_start_line: u64 = 1;
        let mut cur_line: u64 = 0;
        let mut lines_in_block: usize = 0;
        let mut byte: usize = 0;

        for line in text.split_inclusive('\n') {
            cur_line += 1;
            let line_start_byte = byte;
            let is_ts = self.is_timestamp_line(strip_eol(line));

            // A new timestamp (or the size cap) closes the block in progress.
            let start_new = lines_in_block > 0 && (is_ts || lines_in_block >= self.max_lines);
            if start_new {
                let block_text =
                    text[block_start_byte..line_start_byte].trim_end_matches(['\r', '\n']);
                let keep = sink(Unit {
                    text: block_text,
                    line_start: block_start_line,
                    line_end: cur_line - 1,
                    byte_offset: block_start_byte as u64,
                });
                if !keep {
                    return;
                }
                block_start_byte = line_start_byte;
                block_start_line = cur_line;
                lines_in_block = 0;
            }
            lines_in_block += 1;
            byte += line.len();
        }

        // Trailing block.
        if lines_in_block > 0 {
            let block_text = text[block_start_byte..byte].trim_end_matches(['\r', '\n']);
            sink(Unit {
                text: block_text,
                line_start: block_start_line,
                line_end: cur_line,
                byte_offset: block_start_byte as u64,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(splitter: &dyn UnitSplitter, text: &str) -> Vec<(String, u64, u64)> {
        let mut out = Vec::new();
        splitter.split(text, &mut |u| {
            out.push((u.text.to_string(), u.line_start, u.byte_offset));
            true
        });
        out
    }

    #[test]
    fn line_splitter_numbers_and_offsets() {
        let text = "alpha\nbeta\r\ngamma";
        let units = collect(&LineSplitter, text);
        assert_eq!(
            units,
            vec![
                ("alpha".into(), 1, 0),
                ("beta".into(), 2, 6),
                ("gamma".into(), 3, 12),
            ]
        );
    }

    #[test]
    fn line_splitter_early_stop() {
        let text = "one\ntwo\nthree";
        let mut seen = 0;
        LineSplitter.split(text, &mut |_| {
            seen += 1;
            seen < 2
        });
        assert_eq!(seen, 2);
    }

    /// Return (line_start, line_end) for each block.
    fn block_ranges(splitter: &BlockSplitter, text: &str) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        splitter.split(text, &mut |u| {
            out.push((u.line_start, u.line_end));
            true
        });
        out
    }

    #[test]
    fn block_groups_untimestamped_continuation_lines() {
        let text = "2026-01-01 00:00:01 A\n\
                    2026-01-01 00:00:02 B\n\
                    \tstack frame 1\n\
                    \tstack frame 2\n\
                    2026-01-01 00:00:03 C\n";
        let bs = BlockSplitter::default();
        // Block1: line 1; Block2: lines 2-4 (B + 2 frames); Block3: line 5.
        assert_eq!(block_ranges(&bs, text), vec![(1, 1), (2, 4), (5, 5)]);
    }

    #[test]
    fn block_leading_lines_form_orphan_block() {
        let text = "header without timestamp\n\
                    another preamble line\n\
                    2026-01-01 00:00:01 first real entry\n";
        let bs = BlockSplitter::default();
        assert_eq!(block_ranges(&bs, text), vec![(1, 2), (3, 3)]);
    }

    #[test]
    fn block_size_cap_force_splits_untimestamped_run() {
        let bs = BlockSplitter::with_patterns(DEFAULT_TS_PATTERNS, 2).unwrap();
        // One timestamped opener then 3 continuation lines, cap = 2 lines/block.
        let text = "2026-01-01 00:00:01 A\nx\ny\nz\n";
        // [1-2] (A + x), then cap forces [3-4] (y + z).
        assert_eq!(block_ranges(&bs, text), vec![(1, 2), (3, 4)]);
    }

    #[test]
    fn block_recognises_syslog_and_epoch() {
        let bs = BlockSplitter::default();
        assert!(bs.is_timestamp_line("Aug 20 08:00:01 host daemon: msg"));
        assert!(bs.is_timestamp_line("1755676801 event"));
        assert!(bs.is_timestamp_line("2026-08-20T08:00:01 iso"));
        assert!(!bs.is_timestamp_line("    indented continuation line"));
    }
}
