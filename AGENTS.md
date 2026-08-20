# InSearch — developer guide

A real-time, content-aware file search tool: find files on disk and search
inside them, streaming matches per-line or per-timestamp-block. Rust + egui,
Windows-first, Linux-portable.

## Architecture

Cargo **workspace** with a headless core so the engine is testable without a
display, and heavy/native extractor deps stay out of the GUI build.

```
crates/
  insearch-core/   # engine — no GUI, no OS-specific code
    model.rs     # Query, Match, SearchEvent, Granularity, Mode
    extract.rs   # TextExtractor trait + Registry (Raw vs Materialized text)
    split.rs     # UnitSplitter trait; LineSplitter now, BlockSplitter (Phase 2)
    scan.rs      # ignore::WalkParallel + grep-regex/grep-searcher, streaming
  insearch-gui/    # eframe/egui front-end (glow backend only)
    main.rs      # launch + argv path prefill
    app.rs       # state, debounce, worker plumbing, results table
    palette.rs   # light/dark theming
  insearch-cli/    # headless harness + fast test surface
```

### The two-layer extractor design

1. **`extract::TextExtractor`** (format-specific): a file → text. Plain-text/log
   files return `Source::Raw` (grep-searcher streams them directly, getting free
   binary detection + encoding transcoding). Binary formats (Phase 4) decode to
   `Source::Materialized(String)`.
2. **`split::UnitSplitter`** (format-agnostic): text → line/block units. The
   *same* splitter runs over a raw log and over text extracted from a PDF — line
   vs block is a property of the view, not the file format.

Query matching everywhere uses ripgrep's `grep-regex` matcher. The line-mode
plain-text fast path additionally uses `grep-searcher`'s `Searcher`.

### Streaming + cancellation

- `ignore::WalkParallel` provides the thread pool; search runs *inside* the
  per-entry closure (one fused pool, no rayon-over-ignore oversubscription).
- Results stream over a **bounded** `crossbeam-channel` (backpressure).
- Cancellation is a shared `AtomicU64` **generation counter**: starting a new
  search bumps it; stale workers see the mismatch at the next file/result and
  `Quit`. The UI drops events whose generation ≠ the active one. No joins.
- GUI debounces keystrokes (`DEBOUNCE`, `MIN_QUERY_LEN`) and caps the model
  (`MAX_RESULTS`) with a virtualized `egui_extras::TableBuilder`.

## Conventions

- Keep the tree `cargo fmt`-clean and `cargo clippy --all-targets -- -D warnings`-clean.
- Conventional Commits (feat/fix/docs/refactor/perf/test/build/ci/chore). SemVer, pre-1.0.
- Portable code in `insearch-core`; OS-specific code behind `#[cfg(...)]` (context
  menu / registry lands in a `insearch-platform` module in Phase 5).
- `eframe` with the **glow** backend only (no wgpu). Don't hard-pin the patch
  version (`0.36`, not `=0.36.1`).
- Don't commit build artifacts (`/target` is gitignored).

## Binary formats (feature-gated)

`insearch-core` features `xls` (calamine), `docx` (zip + quick-xml), `pdf`
(pdf-extract), and `all-formats`. `extract::default_registry()` registers
whichever are compiled in; the scan closure resolves an extractor per file and
searches `Source::Materialized` text through the same line/block emitters. The
GUI and CLI enable `all-formats`; core's default build stays lean.

## Windows integration

- `crates/insearch-gui/src/context_menu.rs` — HKCU register/unregister of the
  Explorer "Search with InSearch" verbs (`winreg`), driven only by the
  in-app Settings panel. Non-Windows stub returns Unsupported.
- `crates/insearch-gui/build.rs` + `app.rc` + `app.manifest` — embed the Windows
  manifest (PerMonitorV2 DPI, common controls v6, Win10/11 supportedOS) via
  `embed-resource`. Drop an `app.ico` + `ICON` line in `app.rc` to add an icon.

## CI

`.github/workflows/`: `build.yml` (Windows — fmt/clippy/test gate + release exe
+ CLI smoke + artifact/release-attach), `linux.yml` (portable tarball, installs
GTK/wayland/x11/GL), `release-please.yml` (simple release-type → tags vX.Y.Z that
trigger the build workflows). No git remote configured yet.

## Status

All six planned phases shipped. See the plan file (`swift-leaping-lagoon.md`) for
the full history and any deferred items.
