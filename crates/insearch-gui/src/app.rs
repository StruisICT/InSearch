//! The egui application: state, the search worker plumbing, and the UI.
//!
//! Search-as-you-type flow:
//!   * every keystroke stamps `pending_since`;
//!   * once the query has been idle for `DEBOUNCE`, `launch_search` bumps a
//!     shared generation counter (cancelling any prior run) and spawns a worker;
//!   * `ui` drains the results channel each frame and repaints while a
//!     search is live.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use egui_extras::{Column, TableBuilder};

use insearch_core::model::SearchEvent;
use insearch_core::{FileFilter, Granularity, Match, Mode, Query, ScanOptions};

/// Idle time after the last keystroke before a search fires.
const DEBOUNCE: Duration = Duration::from_millis(200);
/// Don't search for very short queries (too many matches, no signal).
const MIN_QUERY_LEN: usize = 2;
/// Cap results held in the UI model (refine the query to narrow further).
const MAX_RESULTS: usize = 10_000;
/// Bounded channel depth — backpressure against a firehose of matches.
const CHANNEL_CAP: usize = 4096;

/// Parse a comma/space/semicolon-separated extension list, dropping any leading
/// dots and lowercasing.
fn parse_exts(s: &str) -> Vec<String> {
    s.split([',', ';', ' ', '\t'])
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .collect()
}

/// Parse a size in kibibytes into bytes; empty/invalid → no bound.
fn parse_kb(s: &str) -> Option<u64> {
    s.trim().parse::<u64>().ok().map(|kb| kb * 1024)
}

/// Parse a positive integer; empty/invalid/zero → `None`.
fn parse_u64(s: &str) -> Option<u64> {
    s.trim().parse::<u64>().ok().filter(|n| *n > 0)
}

pub struct App {
    // Query + options
    query_text: String,
    is_regex: bool,
    granularity: Granularity,
    mode: Mode,
    respect_gitignore: bool,
    dark: bool,

    // File filters (raw UI text, parsed into a FileFilter on search)
    show_filters: bool,
    filter_name: String,
    filter_name_regex: bool,
    filter_include_exts: String,
    filter_exclude_exts: String,
    filter_min_kb: String,
    filter_max_kb: String,
    filter_days: String,

    // Roots to search
    roots: Vec<PathBuf>,

    // Search worker state
    generation_counter: Arc<AtomicU64>,
    active_generation: u64,
    rx: Option<Receiver<SearchEvent>>,
    searching: bool,
    watching: bool,
    watch_handle: Option<insearch_core::WatchHandle>,
    results: Vec<ResultRow>,
    truncated: bool,
    error: Option<String>,

    // Debounce bookkeeping
    pending_since: Option<Instant>,

    // Settings window (Explorer context-menu integration)
    show_settings: bool,
    settings_msg: Option<String>,

    // Watch mode: the watcher is started only after the initial full scan
    // finishes (on `Done`), so its first-seen `Clear` can't race the scan's
    // matches and duplicate rows.
    pending_watch: Option<PendingWatch>,
}

/// Captured parameters for a watcher whose start is deferred until the initial
/// scan for the same generation completes.
struct PendingWatch {
    generation: u64,
    roots: Vec<PathBuf>,
    query: Query,
    generation_counter: Arc<AtomicU64>,
    tx: Sender<SearchEvent>,
}

/// A search hit with its display strings computed once on ingest, so the
/// virtualized results table doesn't reallocate them every frame.
struct ResultRow {
    path: PathBuf,
    file_name: String,
    path_display: String,
    line_label: String,
    line_hover: Option<String>,
    preview: String,
    full_text: String,
}

impl ResultRow {
    fn from_match(m: Match) -> Self {
        let file_name = m
            .path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let path_display = m.path.display().to_string();
        // A multi-line block match shows a line range and a "match on line N"
        // hover; a single-line match shows just the number.
        let (line_label, line_hover) = if m.line_end > m.line_start {
            (
                format!("{}-{}", m.line_start, m.line_end),
                Some(format!("match on line {}", m.matched_line)),
            )
        } else {
            (m.line_start.to_string(), None)
        };
        // Collapse newlines so a block occupies one virtualized row.
        let preview = m.text.replace(['\r', '\n'], " ⏎ ");
        ResultRow {
            path: m.path,
            file_name,
            path_display,
            line_label,
            line_hover,
            preview,
            full_text: m.text,
        }
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_root: Option<PathBuf>) -> Self {
        let dark = true;
        super::palette::apply(&cc.egui_ctx, dark);
        App {
            query_text: String::new(),
            is_regex: false,
            granularity: Granularity::Line,
            mode: Mode::Live,
            respect_gitignore: false,
            dark,
            show_filters: false,
            filter_name: String::new(),
            filter_name_regex: false,
            filter_include_exts: String::new(),
            filter_exclude_exts: String::new(),
            filter_min_kb: String::new(),
            filter_max_kb: String::new(),
            filter_days: String::new(),
            roots: initial_root.into_iter().collect(),
            generation_counter: Arc::new(AtomicU64::new(0)),
            active_generation: 0,
            rx: None,
            searching: false,
            watching: false,
            watch_handle: None,
            results: Vec::new(),
            truncated: false,
            error: None,
            pending_since: None,
            show_settings: false,
            settings_msg: None,
            pending_watch: None,
        }
    }

    fn current_query(&self) -> Query {
        Query {
            pattern: self.query_text.clone(),
            is_regex: self.is_regex,
            smart_case: true,
            granularity: self.granularity,
        }
    }

    fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            respect_gitignore: self.respect_gitignore,
            filter: self.build_filter(),
            ..ScanOptions::default()
        }
    }

    fn build_filter(&self) -> FileFilter {
        FileFilter {
            name_pattern: self.filter_name.trim().to_string(),
            name_is_regex: self.filter_name_regex,
            include_exts: parse_exts(&self.filter_include_exts),
            exclude_exts: parse_exts(&self.filter_exclude_exts),
            min_size: parse_kb(&self.filter_min_kb),
            max_size: parse_kb(&self.filter_max_kb),
            modified_within_days: parse_u64(&self.filter_days),
        }
    }

    /// Cancel any running search/watch and clear results (without restarting).
    fn cancel(&mut self) {
        self.generation_counter.fetch_add(1, Ordering::SeqCst);
        self.searching = false;
        self.watching = false;
        self.watch_handle = None; // drop stops the OS watcher + worker
        self.pending_watch = None;
        self.rx = None;
    }

    /// Bump the generation (cancelling any prior run), run a fresh full scan,
    /// and — in watch mode — start a watcher streaming updates on the same
    /// channel/generation.
    fn launch_search(&mut self) {
        let query = self.current_query();
        self.pending_since = None;
        self.watch_handle = None; // stop any existing watcher before restarting
        self.pending_watch = None;

        // Nothing to search: clear and bail.
        if self.roots.is_empty() || self.query_text.trim().len() < MIN_QUERY_LEN {
            self.cancel();
            self.results.clear();
            self.truncated = false;
            self.error = None;
            return;
        }

        let g = self.generation_counter.fetch_add(1, Ordering::SeqCst) + 1;
        self.active_generation = g;
        self.results.clear();
        self.truncated = false;
        self.error = None;
        self.searching = true;
        self.watching = false;

        let (tx, rx) = crossbeam_channel::bounded(CHANNEL_CAP);
        self.rx = Some(rx);

        let roots = self.roots.clone();
        let opts = self.scan_options();
        let generation_counter = self.generation_counter.clone();

        // Initial full scan (both modes) on its own thread.
        {
            let tx = tx.clone();
            let roots = roots.clone();
            let query = query.clone();
            let generation_counter = generation_counter.clone();
            std::thread::spawn(move || {
                insearch_core::search(&roots, &query, g, generation_counter, opts, tx);
            });
        }

        // Watch mode: defer starting the watcher until the initial scan sends
        // `Done` (see `drain_results`), so the tailer's first-seen `Clear`
        // always post-dates the scan's matches. `tx` here keeps the channel's
        // sender alive until then.
        if self.mode == Mode::Watch {
            self.pending_watch = Some(PendingWatch {
                generation: g,
                roots,
                query,
                generation_counter,
                tx,
            });
        }
    }

    /// Start the deferred watcher once its generation's scan has completed.
    fn start_pending_watch(&mut self) {
        let Some(pw) = self.pending_watch.take() else {
            return;
        };
        if pw.generation != self.active_generation {
            return; // superseded before the scan finished
        }
        match insearch_core::start_watch(
            &pw.roots,
            &pw.query,
            pw.generation,
            pw.generation_counter,
            pw.tx,
        ) {
            Ok(handle) => {
                self.watch_handle = Some(handle);
                self.watching = true;
            }
            Err(e) => self.error = Some(format!("watch: {e}")),
        }
    }

    /// Drain whatever the worker has produced since last frame.
    fn drain_results(&mut self) {
        let events: Vec<SearchEvent> = match &self.rx {
            Some(rx) => rx.try_iter().collect(),
            None => return,
        };
        for ev in events {
            match ev {
                SearchEvent::Match(g, m) => {
                    if g == self.active_generation {
                        if self.results.len() < MAX_RESULTS {
                            self.results.push(ResultRow::from_match(m));
                        } else {
                            self.truncated = true;
                        }
                    }
                }
                SearchEvent::Clear(g, path) => {
                    if g == self.active_generation {
                        self.results.retain(|r| r.path != path);
                        // Removing rows may reopen room under the cap.
                        if self.truncated && self.results.len() < MAX_RESULTS {
                            self.truncated = false;
                        }
                    }
                }
                SearchEvent::Done(g) => {
                    if g == self.active_generation {
                        self.searching = false;
                        // Initial scan finished — safe to begin watching now.
                        self.start_pending_watch();
                    }
                }
                SearchEvent::Error(g, e) => {
                    if g == self.active_generation {
                        self.error = Some(e);
                        self.searching = false;
                    }
                }
            }
        }
    }

    fn add_folder(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            if !self.roots.contains(&dir) {
                self.roots.push(dir);
                self.launch_search();
            }
        }
    }

    /// The collapsible file-filter panel. Any change re-triggers the search.
    fn filters_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::Grid::new("filters_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Name:");
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.filter_name)
                                    .hint_text("glob e.g. *.log")
                                    .desired_width(180.0),
                            )
                            .changed();
                        changed |= ui.checkbox(&mut self.filter_name_regex, "regex").changed();
                    });
                    ui.end_row();

                    ui.label("Extensions:");
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.filter_include_exts)
                                    .hint_text("include e.g. log,txt")
                                    .desired_width(120.0),
                            )
                            .changed();
                        ui.label("exclude:");
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.filter_exclude_exts)
                                    .hint_text("e.g. min.js")
                                    .desired_width(120.0),
                            )
                            .changed();
                    });
                    ui.end_row();

                    ui.label("Size (KB):");
                    ui.horizontal(|ui| {
                        ui.label("min");
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.filter_min_kb)
                                    .desired_width(70.0),
                            )
                            .changed();
                        ui.label("max");
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.filter_max_kb)
                                    .desired_width(70.0),
                            )
                            .changed();
                    });
                    ui.end_row();

                    ui.label("Modified:");
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.filter_days)
                                    .desired_width(70.0),
                            )
                            .changed();
                        ui.label("within N days (blank = any)");
                    });
                    ui.end_row();
                });
        });
        if changed {
            self.pending_since = Some(Instant::now());
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("InSearch");
            ui.separator();
            if ui
                .selectable_label(self.mode == Mode::Live, "Live")
                .on_hover_text("Search-as-you-type; re-scans on each query change")
                .clicked()
                && self.mode != Mode::Live
            {
                self.mode = Mode::Live;
                self.launch_search();
            }
            if ui
                .selectable_label(self.mode == Mode::Watch, "Watch")
                .on_hover_text("Keep results live as files change (log-tailing)")
                .clicked()
                && self.mode != Mode::Watch
            {
                self.mode = Mode::Watch;
                self.launch_search();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(if self.dark { "☀ Light" } else { "🌙 Dark" })
                    .clicked()
                {
                    self.dark = !self.dark;
                    super::palette::apply(ui.ctx(), self.dark);
                }
                if ui.button("⚙ Settings").clicked() {
                    self.show_settings = !self.show_settings;
                    self.settings_msg = None;
                }
            });
        });

        ui.add_space(4.0);

        // Query row.
        ui.horizontal(|ui| {
            ui.label("Search:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.query_text)
                    .hint_text("type to search inside files…")
                    .desired_width(f32::INFINITY),
            );
            if resp.changed() {
                self.pending_since = Some(Instant::now());
            }
        });

        // Options row.
        ui.horizontal(|ui| {
            if ui.checkbox(&mut self.is_regex, "Regex").changed() {
                self.pending_since = Some(Instant::now());
            }
            ui.separator();
            ui.label("Granularity:");
            let mut changed = false;
            changed |= ui
                .selectable_value(&mut self.granularity, Granularity::Line, "Line")
                .on_hover_text("One result per matching line")
                .changed();
            changed |= ui
                .selectable_value(&mut self.granularity, Granularity::Block, "Block")
                .on_hover_text("One result per timestamp-to-timestamp block")
                .changed();
            if changed {
                self.pending_since = Some(Instant::now());
            }
            ui.separator();
            if ui
                .checkbox(&mut self.respect_gitignore, "Respect .gitignore")
                .changed()
            {
                self.pending_since = Some(Instant::now());
            }
            ui.separator();
            ui.toggle_value(&mut self.show_filters, "⚑ Filters");
        });

        // Filters (collapsible).
        if self.show_filters {
            self.filters_ui(ui);
        }

        // Roots row.
        ui.horizontal_wrapped(|ui| {
            ui.label("Folders:");
            if ui.button("➕ Add folder").clicked() {
                self.add_folder();
            }
            let mut remove: Option<usize> = None;
            for (i, r) in self.roots.iter().enumerate() {
                ui.group(|ui| {
                    ui.label(r.display().to_string());
                    if ui.small_button("✖").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                self.roots.remove(i);
                self.launch_search();
            }
        });
    }

    fn status_line(&self) -> String {
        if let Some(e) = &self.error {
            return format!("Error: {e}");
        }
        let base = format!("{} match(es)", self.results.len());
        let more = if self.truncated {
            format!(" (capped at {MAX_RESULTS}; refine query)")
        } else {
            String::new()
        };
        let state = if self.searching {
            " — searching…"
        } else if self.watching {
            " — watching for changes"
        } else {
            ""
        };
        format!("{base}{more}{state}")
    }

    /// Settings window: the Windows Explorer right-click integration toggle.
    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = self.show_settings;
        egui::Window::new("Settings")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.heading("Explorer integration");
                ui.label(
                    "Add a “Search with InSearch” entry to the Windows \
                     right-click menu for files and folders.",
                );
                ui.add_space(6.0);

                let status = super::context_menu::status();
                let (label, color) = match status {
                    super::context_menu::Status::Registered => {
                        ("Installed — points at this app", egui::Color32::GREEN)
                    }
                    super::context_menu::Status::Stale => (
                        "Installed, but points at a different/old copy — re-install to fix",
                        egui::Color32::YELLOW,
                    ),
                    super::context_menu::Status::NotRegistered => {
                        ("Not installed", egui::Color32::GRAY)
                    }
                    super::context_menu::Status::Error => {
                        ("Status unavailable", egui::Color32::LIGHT_RED)
                    }
                };
                ui.colored_label(color, label);
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    let install_text = if status == super::context_menu::Status::Stale {
                        "Re-install entry"
                    } else {
                        "Install entry"
                    };
                    if ui.button(install_text).clicked() {
                        self.settings_msg = Some(match super::context_menu::register() {
                            Ok(()) => "Installed. Right-click a file or folder in Explorer.".into(),
                            Err(e) => format!("Failed: {e}"),
                        });
                    }
                    if ui.button("Remove entry").clicked() {
                        self.settings_msg = Some(match super::context_menu::unregister() {
                            Ok(()) => "Removed.".into(),
                            Err(e) => format!("Failed: {e}"),
                        });
                    }
                });

                if !cfg!(windows) {
                    ui.add_space(4.0);
                    ui.weak("(The context menu is only available on Windows.)");
                }
                if let Some(msg) = &self.settings_msg {
                    ui.add_space(6.0);
                    ui.label(msg);
                }
            });
        self.show_settings = open;
    }

    fn results_table(&self, ui: &mut egui::Ui) {
        let text_height = egui::TextStyle::Body.resolve(ui.style()).size + 6.0;
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .column(Column::auto().at_least(160.0)) // file
            .column(Column::auto().at_least(48.0)) // line
            .column(Column::remainder()) // text
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong("File");
                });
                header.col(|ui| {
                    ui.strong("Line");
                });
                header.col(|ui| {
                    ui.strong("Match");
                });
            })
            .body(|body| {
                body.rows(text_height, self.results.len(), |mut row| {
                    let r = &self.results[row.index()];
                    row.col(|ui| {
                        ui.label(&r.file_name).on_hover_text(&r.path_display);
                    });
                    row.col(|ui| {
                        let resp = ui.monospace(&r.line_label);
                        if let Some(hover) = &r.line_hover {
                            resp.on_hover_text(hover);
                        }
                    });
                    row.col(|ui| {
                        ui.label(&r.preview).on_hover_text(&r.full_text);
                    });
                });
            });
    }
}

impl eframe::App for App {
    // egui 0.36: the App entry point is `ui` (a root `Ui`), and panels are shown
    // *into* a `Ui` rather than onto the `Context`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Fire a debounced search once typing settles.
        if let Some(since) = self.pending_since {
            if since.elapsed() >= DEBOUNCE {
                self.launch_search(); // clears pending_since
            } else {
                ctx.request_repaint_after(DEBOUNCE);
            }
        }

        self.drain_results();

        egui::Panel::top("top").show(ui, |ui| {
            self.top_bar(ui);
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(self.status_line());
            });
        });

        self.settings_window(&ctx);

        egui::CentralPanel::default().show(ui, |ui| {
            if self.results.is_empty() && !self.searching {
                ui.centered_and_justified(|ui| {
                    ui.weak(if self.roots.is_empty() {
                        "Add a folder, then type a query."
                    } else {
                        "Type at least two characters to search."
                    });
                });
            } else {
                self.results_table(ui);
            }
        });

        // Keep draining while a search streams in (fast) or a watch is active
        // (slower poll — watch updates are low-frequency).
        if self.searching {
            ctx.request_repaint_after(Duration::from_millis(50));
        } else if self.watching {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}
