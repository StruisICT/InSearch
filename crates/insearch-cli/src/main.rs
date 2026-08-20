//! Headless search harness. Also the fast surface for exercising the engine.
//!
//! Usage: `insearch-cli <pattern> <root> [more roots...] [--regex] [--gitignore]`

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use insearch_core::{search_collect, Query, ScanOptions};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!(
            "InSearch CLI\n\n\
             Usage: insearch-cli <pattern> <root> [more roots...] [--regex] [--gitignore]\n\n\
             Options:\n\
             \t--regex      treat <pattern> as a regular expression\n\
             \t--gitignore  honour .gitignore / hidden-file rules (default: search everything)"
        );
        return ExitCode::from(2);
    }

    let mut pattern: Option<String> = None;
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut is_regex = false;
    let mut opts = ScanOptions::default();

    for arg in args {
        match arg.as_str() {
            "--regex" => is_regex = true,
            "--gitignore" => opts.respect_gitignore = true,
            other if pattern.is_none() => pattern = Some(other.to_string()),
            other => roots.push(PathBuf::from(other)),
        }
    }

    let pattern = pattern.unwrap();
    if roots.is_empty() {
        roots.push(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    }

    let query = Query {
        pattern,
        is_regex,
        smart_case: true,
        granularity: insearch_core::Granularity::Line,
    };

    let hits = search_collect(&roots, &query, opts);

    // Write through a buffered, locked stdout and stop quietly if the reader
    // closes the pipe (e.g. `insearch-cli ... | head` / `| grep -q`), rather
    // than panicking like `println!` would.
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    for m in &hits {
        if writeln!(out, "{}:{}: {}", m.path.display(), m.line_start, m.text).is_err() {
            return ExitCode::SUCCESS;
        }
    }
    if out.flush().is_err() {
        return ExitCode::SUCCESS;
    }
    eprintln!("{} match(es).", hits.len());

    if hits.is_empty() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
