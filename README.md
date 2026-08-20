# Inspector Fetchy

> Working title — will change later.

A real-time, content-aware search tool. It finds files on disk **and** searches
*inside* them, streaming matches as it goes. Built for logs and documents: search
`txt`, `xml`, `json` and other text files now; `pdf` / `docx` / `xls` later.
Matches can be reported **per line** or **per timestamp-to-timestamp block**.

Rust + [egui](https://github.com/emilk/egui). Windows-first, Linux-portable.

## Status

Working. Core search, block mode, watch mode, and binary extractors are in.

- [x] Streaming live search-as-you-type (line mode) over chosen folders
- [x] Block / timestamp extraction (per-block matches)
- [x] Watch-folders mode (log-tailing)
- [x] Binary extractors: xls / xlsx / ods, docx, pdf (behind cargo features)
- [x] Windows Explorer "Search with Inspector Fetchy" context menu (opt-in via Settings)
- [ ] CI / packaging

Enable the Explorer right-click entry from **⚙ Settings → Explorer integration**
(installs under `HKCU`, no admin required; removable from the same panel).

### Cargo features

Binary-format support is opt-in on `fetchy-core` (the GUI and CLI enable
`all-formats`): `xls` (calamine), `docx` (zip + quick-xml), `pdf` (pdf-extract),
and `all-formats` for all three.

## Build & run

```sh
# GUI
cargo run -p fetchy-gui              # optional: ... -- "C:\path\to\search"

# CLI
cargo run -p fetchy-cli -- <pattern> <root> [more roots...] [--regex] [--gitignore]

# Tests
cargo test -p fetchy-core
```

## Workspace layout

| Crate         | Role                                                                 |
|---------------|---------------------------------------------------------------------|
| `fetchy-core` | Headless engine: walk (`ignore`), match (`grep-regex`), extract, split |
| `fetchy-gui`  | egui/eframe desktop front-end (glow backend)                        |
| `fetchy-cli`  | Command-line harness (also the fast test surface)                   |

See [`AGENTS.md`](AGENTS.md) for architecture and conventions.

## License

MIT © Struis ICT
