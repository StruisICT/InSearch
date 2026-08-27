//! Headless search harness and scripting surface for the InSearch engine.
//!
//! Exposes the engine's query modes, file filters, the in-content entry-time
//! filter, and a few output formats. Run `insearch-cli --help` for the flags.

use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use insearch_core::timestamp::TimestampParser;
use insearch_core::{
    search_collect, CaseMode, FileFilter, Granularity, MatchMode, Query, ScanOptions, TimeFilter,
};

const HELP: &str = "\
InSearch CLI — find files and search inside them.

Usage: insearch-cli <pattern> [roots...] [options]
  <pattern>   text to search for (substring by default)
  roots       one or more files/folders (default: current directory)

Query:
  --regex             treat <pattern> as a regular expression
  --all-words         every space-separated word must appear (AND)
  --any-words         any of the space-separated words may appear (OR)
  --exclude \"a b\"      words that must NOT appear (NOT)
  --whole-word        match whole words only
  --case-sensitive    force case-sensitive (default: smart case)
  --ignore-case       force case-insensitive
  --block             report one result per timestamp-to-timestamp block
  --gitignore         honour .gitignore / hidden-file rules (default: search all)

File filters:
  --name <glob>       filename glob, e.g. *.log
  --name-regex        treat --name as a regex
  --ext <a,b>         only these extensions
  --exclude-ext <a,b> not these extensions
  --min-kb <n>        minimum size (KB)
  --max-kb <n>        maximum size (KB)
  --days <n>          modified within the last n days
  --after <when>      file modified at/after <when>
  --before <when>     file modified at/before <when>

Entry-time filter (timestamp inside each matching line/block):
  --entry-after <when>   keep entries at/after <when>
  --entry-before <when>  keep entries at/before <when>
  (<when> is YYYY-MM-DD or 'YYYY-MM-DD HH:MM:SS', local time)

Output:
  --json              one JSON object per line ({\"path\",\"line\",\"text\"})
  --count             print only the number of matches
  -l, --files-with-matches   print only the matching file paths (unique)
  -h, --help          show this help";

/// What to print for the results.
#[derive(Clone, Copy, PartialEq)]
enum Output {
    Lines,
    Json,
    Count,
    FilesOnly,
}

/// A fully-parsed invocation.
struct Config {
    query: Query,
    roots: Vec<PathBuf>,
    opts: ScanOptions,
    output: Output,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        // Showing usage is a successful outcome (exit 0), not an error.
        eprintln!("{HELP}");
        return ExitCode::SUCCESS;
    }

    let cfg = match parse(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}\n\nRun `insearch-cli --help` for usage.");
            return ExitCode::from(2);
        }
    };

    let hits = search_collect(&cfg.roots, &cfg.query, cfg.opts);

    // Buffered, locked stdout; stop quietly if the reader closes the pipe
    // (e.g. `... | head`) rather than panicking like `println!` would.
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let write_ok = match cfg.output {
        Output::Count => writeln!(out, "{}", hits.len()).is_ok(),
        Output::FilesOnly => {
            let paths: BTreeSet<String> =
                hits.iter().map(|m| m.path.display().to_string()).collect();
            paths.iter().try_for_each(|p| writeln!(out, "{p}")).is_ok()
        }
        Output::Json => hits
            .iter()
            .try_for_each(|m| {
                writeln!(
                    out,
                    "{{\"path\":\"{}\",\"line\":{},\"text\":\"{}\"}}",
                    json_escape(&m.path.display().to_string()),
                    m.line_start,
                    json_escape(&m.text),
                )
            })
            .is_ok(),
        Output::Lines => hits
            .iter()
            .try_for_each(|m| writeln!(out, "{}:{}: {}", m.path.display(), m.line_start, m.text))
            .is_ok(),
    };
    if !write_ok || out.flush().is_err() {
        return ExitCode::SUCCESS; // broken pipe — done
    }
    if cfg.output != Output::Count {
        eprintln!("{} match(es).", hits.len());
    }

    if hits.is_empty() {
        ExitCode::from(1) // grep convention: no matches
    } else {
        ExitCode::SUCCESS
    }
}

/// Parse argv (excluding the program name) into a [`Config`].
fn parse(args: &[String]) -> Result<Config, String> {
    let mut pattern: Option<String> = None;
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut mode = MatchMode::Substring;
    let mut case = CaseMode::Smart;
    let mut whole_word = false;
    let mut granularity = Granularity::Line;
    let mut exclude = String::new();
    let mut opts = ScanOptions::default();
    let mut output = Output::Lines;
    let mut filter = FileFilter::default();
    let mut after: Option<i64> = None;
    let mut before: Option<i64> = None;
    let parser = TimestampParser::default();

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--regex" => mode = MatchMode::Regex,
            "--all-words" => mode = MatchMode::AllWords,
            "--any-words" => mode = MatchMode::AnyWords,
            "--case-sensitive" => case = CaseMode::Sensitive,
            "--ignore-case" => case = CaseMode::Insensitive,
            "--whole-word" => whole_word = true,
            "--block" => granularity = Granularity::Block,
            "--gitignore" => opts.respect_gitignore = true,
            "--json" => output = Output::Json,
            "--count" => output = Output::Count,
            "-l" | "--files-with-matches" => output = Output::FilesOnly,
            "--exclude" => exclude = take(args, &mut i, a)?,
            "--name" => filter.name_pattern = take(args, &mut i, a)?,
            "--name-regex" => filter.name_is_regex = true,
            "--ext" => filter.include_exts = parse_exts(&take(args, &mut i, a)?),
            "--exclude-ext" => filter.exclude_exts = parse_exts(&take(args, &mut i, a)?),
            "--min-kb" => filter.min_size = Some(parse_u64(&take(args, &mut i, a)?, a)? * 1024),
            "--max-kb" => filter.max_size = Some(parse_u64(&take(args, &mut i, a)?, a)? * 1024),
            "--days" => filter.modified_within_days = Some(parse_u64(&take(args, &mut i, a)?, a)?),
            "--after" => {
                filter.modified_after = Some(to_time(parse_when(&parser, &take(args, &mut i, a)?)?))
            }
            "--before" => {
                filter.modified_before =
                    Some(to_time(parse_when(&parser, &take(args, &mut i, a)?)?))
            }
            "--entry-after" => after = Some(parse_when(&parser, &take(args, &mut i, a)?)?),
            "--entry-before" => before = Some(parse_when(&parser, &take(args, &mut i, a)?)?),
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown flag: {other}"));
            }
            other if pattern.is_none() => pattern = Some(other.to_string()),
            other => roots.push(PathBuf::from(other)),
        }
        i += 1;
    }

    let pattern = pattern.ok_or("no search pattern given")?;
    if roots.is_empty() {
        roots.push(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    }
    if after.is_some() || before.is_some() {
        opts.time = Some(TimeFilter {
            after,
            before,
            mtime_prefilter: false,
        });
    }
    opts.filter = filter;

    Ok(Config {
        query: Query {
            pattern,
            exclude,
            mode,
            case,
            whole_word,
            granularity,
        },
        roots,
        opts,
        output,
    })
}

/// Consume the value that follows a value-taking flag, advancing the index.
fn take(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

/// Comma/space/semicolon-separated extension list, lowercased, dots stripped.
fn parse_exts(s: &str) -> Vec<String> {
    s.split([',', ';', ' ', '\t'])
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .collect()
}

fn parse_u64(s: &str, flag: &str) -> Result<u64, String> {
    s.trim()
        .parse::<u64>()
        .map_err(|_| format!("{flag} expects a number, got '{s}'"))
}

/// Parse `YYYY-MM-DD` or `YYYY-MM-DD HH:MM:SS` (local) into epoch seconds, via
/// the engine's own timestamp parser.
fn parse_when(parser: &TimestampParser, v: &str) -> Result<i64, String> {
    let v = v.trim();
    let candidate = if v.len() <= 10 {
        format!("{v} 00:00:00")
    } else {
        v.to_string()
    };
    parser
        .parse_leading(&candidate)
        .ok_or_else(|| format!("could not parse date/time: '{v}' (use YYYY-MM-DD [HH:MM:SS])"))
}

/// Epoch seconds → `SystemTime` (for the file-date filter).
fn to_time(epoch: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(epoch.max(0) as u64)
}

/// Minimal JSON string escaping (no serde dependency for one output mode).
fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_query_and_filters() {
        let cfg = parse(&args(&[
            "ERROR",
            "logs",
            "--regex",
            "--ignore-case",
            "--whole-word",
            "--block",
            "--exclude",
            "debug trace",
            "--ext",
            "log,txt",
            "--min-kb",
            "2",
            "--json",
        ]))
        .unwrap();
        assert_eq!(cfg.query.pattern, "ERROR");
        assert!(matches!(cfg.query.mode, MatchMode::Regex));
        assert!(matches!(cfg.query.case, CaseMode::Insensitive));
        assert!(cfg.query.whole_word);
        assert!(matches!(cfg.query.granularity, Granularity::Block));
        assert_eq!(cfg.query.exclude, "debug trace");
        assert_eq!(cfg.opts.filter.include_exts, vec!["log", "txt"]);
        assert_eq!(cfg.opts.filter.min_size, Some(2048));
        assert!(cfg.output == Output::Json);
        assert_eq!(cfg.roots.len(), 1);
    }

    #[test]
    fn entry_time_bounds_set_the_time_filter() {
        let cfg = parse(&args(&["x", ".", "--entry-after", "2026-08-21"])).unwrap();
        let t = cfg.opts.time.expect("time filter");
        assert!(t.after.is_some());
        assert!(t.before.is_none());
    }

    #[test]
    fn rejects_unknown_flag_and_missing_value() {
        assert!(parse(&args(&["x", "--bogus"])).is_err());
        assert!(parse(&args(&["x", "--ext"])).is_err()); // needs a value
        assert!(parse(&args(&["--regex"])).is_err()); // no pattern
    }

    #[test]
    fn json_escape_handles_quotes_and_controls() {
        assert_eq!(json_escape("a\"b\\c\n"), "a\\\"b\\\\c\\n");
    }
}
