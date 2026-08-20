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

use crossbeam_channel::Receiver;
use eframe::egui;
use egui_extras::{Column, TableBuilder};

use insearch_core::model::SearchEvent;
use insearch_core::{Granularity, Match, Mode, Query, ScanOptions};

/// Idle time after the last keystroke before a search fires.
const DEBOUNCE: Duration = Duration::from_millis(200);
/// Don't search for very short queries (too many matches, no signal).
const MIN_QUERY_LEN: usize = 2;
/// Cap results held in the UI model (refine the query to narrow further).
const MAX_RESULTS: usize = 10_000;
/// Bounded channel depth — backpressure against a firehose of matches.
const CHANNEL_CAP: usize = 4096;

pub struct App {
    // Query + options
    query_text: String,
    is_regex: bool,
    granularity: Granularity,
    mode: Mode,
    respect_gitignore: bool,
    dark: bool,

    // Roots to search
    roots: Vec<PathBuf>,

    // Search worker state
    gen_counter: Arc<AtomicU64>,
    active_gen: u64,
    rx: Option<Receiver<SearchEvent>>,
    searching: bool,
    watching: bool,
    watch_handle: Option<insearch_core::WatchHandle>,
    results: Vec<Match>,
    truncated: bool,
    error: Option<String>,

    // Debounce bookkeeping
    pending_since: Option<Instant>,

    // Settings window (Explorer context-menu integration)
    show_settings: bool,
    settings_msg: Option<String>,
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
            roots: initial_root.into_iter().collect(),
            gen_counter: Arc::new(AtomicU64::new(0)),
            active_gen: 0,
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
            ..ScanOptions::default()
        }
    }

    /// Cancel any running search/watch and clear results (without restarting).
    fn cancel(&mut self) {
        self.gen_counter.fetch_add(1, Ordering::SeqCst);
        self.searching = false;
        self.watching = false;
        self.watch_handle = None; // drop stops the OS watcher + worker
        self.rx = None;
    }

    /// Bump the generation (cancelling any prior run), run a fresh full scan,
    /// and — in watch mode — start a watcher streaming updates on the same
    /// channel/generation.
    fn launch_search(&mut self) {
        let query = self.current_query();
        self.pending_since = None;
        self.watch_handle = None; // stop any existing watcher before restarting

        // Nothing to search: clear and bail.
        if self.roots.is_empty() || self.query_text.trim().len() < MIN_QUERY_LEN {
            self.cancel();
            self.results.clear();
            self.truncated = false;
            self.error = None;
            return;
        }

        let g = self.gen_counter.fetch_add(1, Ordering::SeqCst) + 1;
        self.active_gen = g;
        self.results.clear();
        self.truncated = false;
        self.error = None;
        self.searching = true;
        self.watching = false;

        let (tx, rx) = crossbeam_channel::bounded(CHANNEL_CAP);
        self.rx = Some(rx);

        let roots = self.roots.clone();
        let opts = self.scan_options();
        let gen_counter = self.gen_counter.clone();

        // Initial full scan (both modes) on its own thread.
        {
            let tx = tx.clone();
            let roots = roots.clone();
            let query = query.clone();
            let gen_counter = gen_counter.clone();
            std::thread::spawn(move || {
                insearch_core::search(&roots, &query, g, gen_counter, opts, tx);
            });
        }

        // Watch mode: stream incremental updates on the same channel.
        if self.mode == Mode::Watch {
            match insearch_core::start_watch(&roots, &query, g, gen_counter, tx) {
                Ok(handle) => {
                    self.watch_handle = Some(handle);
                    self.watching = true;
                }
                Err(e) => self.error = Some(format!("watch: {e}")),
            }
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
                    if g == self.active_gen {
                        if self.results.len() < MAX_RESULTS {
                            self.results.push(m);
                        } else {
                            self.truncated = true;
                        }
                    }
                }
                SearchEvent::Clear(g, path) => {
                    if g == self.active_gen {
                        self.results.retain(|m| m.path != path);
                        // Removing rows may reopen room under the cap.
                        if self.truncated && self.results.len() < MAX_RESULTS {
                            self.truncated = false;
                        }
                    }
                }
                SearchEvent::Done(g) => {
                    if g == self.active_gen {
                        self.searching = false;
                    }
                }
                SearchEvent::Error(g, e) => {
                    if g == self.active_gen {
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
        });

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
                    let m = &self.results[row.index()];
                    row.col(|ui| {
                        let name = m
                            .path
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        ui.label(name).on_hover_text(m.path.display().to_string());
                    });
                    row.col(|ui| {
                        // Show a range for multi-line block matches; a single
                        // number for line matches.
                        if m.line_end > m.line_start {
                            ui.monospace(format!("{}-{}", m.line_start, m.line_end))
                                .on_hover_text(format!("match on line {}", m.matched_line));
                        } else {
                            ui.monospace(m.line_start.to_string());
                        }
                    });
                    row.col(|ui| {
                        // Collapse newlines so a block occupies one virtualized row.
                        let flat = m.text.replace(['\r', '\n'], " ⏎ ");
                        ui.label(flat).on_hover_text(&m.text);
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
