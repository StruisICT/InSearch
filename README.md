# InSearch

A real-time, content-aware search tool. It finds files on disk **and** searches
*inside* them, streaming matches as it goes. Built for logs and documents: search
`txt`, `xml`, `json` and other text files now; `pdf` / `docx` / `xls` later.
Matches can be reported **per line** or **per timestamp-to-timestamp block**.

Rust + [egui](https://github.com/emilk/egui). Windows-first, Linux-portable.

## Status

Working, and reasonably featureful.

**Search**
- [x] Streaming live search-as-you-type over chosen folders
- [x] Line mode and timestamp-to-timestamp **block** mode
- [x] Watch-folders mode (log-tailing)
- [x] Query power: substring / regex / **all words** (AND) / **any words** (OR),
      an **exclude** field (NOT), whole-word, and case modes (smart/sensitive/ignore)
- [x] Match highlighting in results

**Files & formats**
- [x] Binary extractors: xls / xlsx / ods, docx, pdf (behind cargo features)
- [x] File filters: name glob/regex, extension include/exclude, size, modified-within-N-days

**Results & session**
- [x] Double-click to open · right-click to reveal / copy path / copy text
- [x] Export results to CSV / JSON / plain text
- [x] In-place "filter results" box
- [x] Recent queries + saved searches (persisted); live progress (files scanned, elapsed)

**Integration**
- [x] Windows Explorer "Search with InSearch" context menu (opt-in via ⚙ Settings)
- [x] CI (Windows + Linux build, release-please)

Enable the Explorer right-click entry from **⚙ Settings → Explorer integration**
(installs under `HKCU`, no admin required; removable from the same panel).

### Cargo features

Binary-format support is opt-in on `insearch-core` (the GUI and CLI enable
`all-formats`): `xls` (calamine), `docx` (zip + quick-xml), `pdf` (pdf-extract),
and `all-formats` for all three.

## Build & run

```sh
# GUI
cargo run -p insearch-gui              # optional: ... -- "C:\path\to\search"

# CLI
cargo run -p insearch-cli -- <pattern> <root> [more roots...] [--regex] [--gitignore]

# Tests
cargo test -p insearch-core
```

## Workspace layout

| Crate         | Role                                                                 |
|---------------|---------------------------------------------------------------------|
| `insearch-core` | Headless engine: walk (`ignore`), match (`grep-regex`), extract, split |
| `insearch-gui`  | egui/eframe desktop front-end (glow backend)                        |
| `insearch-cli`  | Command-line harness (also the fast test surface)                   |

See [`AGENTS.md`](AGENTS.md) for architecture and conventions.

## License

MIT © Struis ICT
