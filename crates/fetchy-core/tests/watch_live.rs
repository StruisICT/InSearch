//! Live end-to-end check of the real `notify` watch path. Ignored by default
//! (timing-dependent); run explicitly with:
//!   cargo test -p fetchy-core --test watch_live -- --ignored

use std::io::Write;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fetchy_core::model::SearchEvent;
use fetchy_core::{start_watch, Granularity, Query};

#[test]
#[ignore = "timing-dependent; run with --ignored"]
fn watcher_reports_appended_matches() {
    let dir = std::env::temp_dir().join(format!("fetchy-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("live.log");
    std::fs::write(&path, "2026-01-01 00:00:00 INFO boot\n").unwrap();

    let (tx, rx) = crossbeam_channel::bounded(1024);
    let current_gen = Arc::new(AtomicU64::new(1));
    let query = Query {
        pattern: "ERROR".into(),
        is_regex: false,
        smart_case: true,
        granularity: Granularity::Line,
    };

    let _handle = start_watch(std::slice::from_ref(&dir), &query, 1, current_gen, tx).unwrap();

    // Give the watcher a moment to register, then append a matching line.
    std::thread::sleep(Duration::from_millis(300));
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(b"2026-01-01 00:00:01 ERROR kaboom\n").unwrap();
        f.flush().unwrap();
    }

    // Wait up to ~5s (debounce is 500ms) for the match to arrive.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got_match = false;
    while Instant::now() < deadline {
        if let Ok(SearchEvent::Match(_, m)) = rx.recv_timeout(Duration::from_millis(200)) {
            if m.text.contains("kaboom") {
                got_match = true;
                break;
            }
        }
    }

    std::fs::remove_dir_all(&dir).ok();
    assert!(got_match, "watcher did not report the appended ERROR line");
}
