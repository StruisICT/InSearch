//! Layer 1 of the extractor design: turning a file into text.
//!
//! Plain-text/log files are handed back as a [`Source::Raw`] path so the scan
//! engine can let grep-searcher mmap/stream them directly (free binary
//! detection + encoding transcoding). Future binary formats (pdf/docx/xls) will
//! decode to a [`Source::Materialized`] `String` that the same
//! [`split`](crate::split) layer then divides into units.

use std::path::Path;

/// Where a file's searchable text lives.
#[derive(Debug)]
pub enum Source {
    /// Search the file on disk as-is (plain text / logs). Carries no bytes;
    /// the engine streams the path.
    Raw,
    /// Text already decoded into memory (used by binary-format extractors).
    Materialized(String),
}

/// Turns a specific family of files into searchable text.
pub trait TextExtractor: Send + Sync {
    /// Lowercase extensions this extractor claims (without the dot).
    fn extensions(&self) -> &'static [&'static str];

    /// Produce the file's text. `Ok(None)` means "skip this file"
    /// (unsupported / extraction failed / not worth searching).
    fn extract(&self, path: &Path) -> std::io::Result<Option<Source>>;
}

/// The default extractor: treat the file as raw text and let grep-searcher do
/// binary detection. It claims a set of known-text extensions but the registry
/// also uses it as the fallback for unknown extensions.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlainTextExtractor;

/// Extensions we confidently treat as plain text.
pub const PLAIN_TEXT_EXTS: &[&str] = &[
    "txt",
    "log",
    "xml",
    "json",
    "csv",
    "tsv",
    "md",
    "yml",
    "yaml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "html",
    "htm",
    "css",
    "js",
    "ts",
    "rs",
    "py",
    "java",
    "c",
    "h",
    "cpp",
    "hpp",
    "cs",
    "go",
    "rb",
    "php",
    "sh",
    "bat",
    "ps1",
    "sql",
    "properties",
];

impl TextExtractor for PlainTextExtractor {
    fn extensions(&self) -> &'static [&'static str] {
        PLAIN_TEXT_EXTS
    }

    fn extract(&self, _path: &Path) -> std::io::Result<Option<Source>> {
        Ok(Some(Source::Raw))
    }
}

/// Maps file extensions to the extractor that handles them.
///
/// Binary-format extractors register here in Phase 4. Until then everything
/// resolves to [`PlainTextExtractor`], and files with unknown extensions are
/// still searched as raw text (grep-searcher skips true binaries).
pub struct Registry {
    plain: PlainTextExtractor,
    /// Format-specific extractors (Phase 4). Each is stored once; the extension
    /// index below points into this vec.
    extractors: Vec<Box<dyn TextExtractor>>,
    /// Lowercase extension -> index into `extractors`.
    by_ext: Vec<(String, usize)>,
}

impl Registry {
    /// A registry with only the plain-text fallback.
    pub fn with_defaults() -> Self {
        Registry {
            plain: PlainTextExtractor,
            extractors: Vec::new(),
            by_ext: Vec::new(),
        }
    }

    /// Register a format-specific extractor for all of its extensions. A later
    /// registration for the same extension wins.
    pub fn register(&mut self, extractor: Box<dyn TextExtractor>) {
        let idx = self.extractors.len();
        for ext in extractor.extensions() {
            let ext = ext.to_ascii_lowercase();
            if let Some(slot) = self.by_ext.iter_mut().find(|(k, _)| *k == ext) {
                slot.1 = idx;
            } else {
                self.by_ext.push((ext, idx));
            }
        }
        self.extractors.push(extractor);
    }

    /// Resolve the extractor for a path by its extension. Falls back to the
    /// plain-text extractor for unknown/absent extensions.
    pub fn resolve(&self, path: &Path) -> &dyn TextExtractor {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext = ext.to_ascii_lowercase();
            if let Some((_, idx)) = self.by_ext.iter().find(|(k, _)| *k == ext) {
                return self.extractors[*idx].as_ref();
            }
        }
        &self.plain
    }

    /// Whether the extension is one we explicitly recognise as plain text.
    pub fn is_known_text(ext: &str) -> bool {
        PLAIN_TEXT_EXTS.contains(&ext.to_ascii_lowercase().as_str())
    }
}

impl Default for Registry {
    fn default() -> Self {
        Registry::with_defaults()
    }
}

/// Build a registry with whichever binary-format extractors were compiled in
/// (via the `xls` / `docx` / `pdf` features). Without those features every file
/// resolves to the plain-text fallback.
pub fn default_registry() -> Registry {
    #[allow(unused_mut)]
    let mut reg = Registry::with_defaults();
    #[cfg(feature = "xls")]
    reg.register(Box::new(formats::SpreadsheetExtractor));
    #[cfg(feature = "docx")]
    reg.register(Box::new(formats::DocxExtractor));
    #[cfg(feature = "pdf")]
    reg.register(Box::new(formats::PdfExtractor));
    reg
}

/// Feature-gated extractors for binary document formats. Each decodes to a
/// [`Source::Materialized`] string that the shared [`split`](crate::split) layer
/// then divides into line/block units, exactly like a plain-text file. Any
/// extraction failure yields `Ok(None)` (skip) so one bad file can't abort a scan.
pub mod formats {
    #[allow(unused_imports)]
    use super::{Source, TextExtractor};
    #[allow(unused_imports)]
    use std::path::Path;

    /// xls/xlsx/xlsb/ods via `calamine` (pure Rust). Rows become tab-separated
    /// lines, sheets concatenated.
    #[cfg(feature = "xls")]
    pub struct SpreadsheetExtractor;

    #[cfg(feature = "xls")]
    impl TextExtractor for SpreadsheetExtractor {
        fn extensions(&self) -> &'static [&'static str] {
            &["xls", "xlsx", "xlsb", "ods"]
        }

        fn extract(&self, path: &Path) -> std::io::Result<Option<Source>> {
            use calamine::{open_workbook_auto, Reader};
            use std::fmt::Write as _;
            let mut wb = match open_workbook_auto(path) {
                Ok(w) => w,
                Err(_) => return Ok(None),
            };
            let mut out = String::new();
            for name in wb.sheet_names() {
                if let Ok(range) = wb.worksheet_range(&name) {
                    for row in range.rows() {
                        for (i, cell) in row.iter().enumerate() {
                            if i > 0 {
                                out.push('\t');
                            }
                            let _ = write!(out, "{cell}");
                        }
                        out.push('\n');
                    }
                }
            }
            Ok(Some(Source::Materialized(out)))
        }
    }

    /// docx via `zip` + `quick-xml`: pull text runs from `word/document.xml`,
    /// one line per paragraph.
    #[cfg(feature = "docx")]
    pub struct DocxExtractor;

    #[cfg(feature = "docx")]
    impl TextExtractor for DocxExtractor {
        fn extensions(&self) -> &'static [&'static str] {
            &["docx"]
        }

        fn extract(&self, path: &Path) -> std::io::Result<Option<Source>> {
            use std::io::Read as _;
            let file = match std::fs::File::open(path) {
                Ok(f) => f,
                Err(_) => return Ok(None),
            };
            let mut zip = match zip::ZipArchive::new(file) {
                Ok(z) => z,
                Err(_) => return Ok(None),
            };
            let mut xml = String::new();
            {
                let mut entry = match zip.by_name("word/document.xml") {
                    Ok(e) => e,
                    Err(_) => return Ok(None),
                };
                if entry.read_to_string(&mut xml).is_err() {
                    return Ok(None);
                }
            }
            Ok(Some(Source::Materialized(docx_xml_to_text(&xml))))
        }
    }

    #[cfg(feature = "docx")]
    pub(crate) fn docx_xml_to_text(xml: &str) -> String {
        use quick_xml::events::Event;
        use quick_xml::Reader;
        let mut reader = Reader::from_str(xml);
        let mut out = String::new();
        let mut in_text = false;
        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) if e.name().as_ref() == b"w:t" => in_text = true,
                Ok(Event::End(e)) => match e.name().as_ref() {
                    b"w:t" => in_text = false,
                    b"w:p" => out.push('\n'), // paragraph -> newline
                    _ => {}
                },
                Ok(Event::Empty(e)) => match e.name().as_ref() {
                    b"w:br" | b"w:cr" => out.push('\n'),
                    b"w:tab" => out.push('\t'),
                    _ => {}
                },
                Ok(Event::Text(t)) if in_text => {
                    if let Ok(s) = t.decode() {
                        out.push_str(&s);
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        out
    }

    /// pdf via `pdf-extract` (lopdf-based, pure Rust). Extraction quality varies
    /// by producer; failures skip the file rather than error.
    #[cfg(feature = "pdf")]
    pub struct PdfExtractor;

    #[cfg(feature = "pdf")]
    impl TextExtractor for PdfExtractor {
        fn extensions(&self) -> &'static [&'static str] {
            &["pdf"]
        }

        fn extract(&self, path: &Path) -> std::io::Result<Option<Source>> {
            match pdf_extract::extract_text(path) {
                Ok(text) => Ok(Some(Source::Materialized(text))),
                Err(_) => Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn resolves_plain_text_for_known_and_unknown() {
        let reg = Registry::with_defaults();
        // Known text extension.
        let e = reg.resolve(Path::new("app.log"));
        assert!(matches!(
            e.extract(Path::new("app.log")).unwrap(),
            Some(Source::Raw)
        ));
        // Unknown extension still falls back to raw.
        let e = reg.resolve(Path::new("mystery.weirdext"));
        assert!(matches!(
            e.extract(Path::new("mystery.weirdext")).unwrap(),
            Some(Source::Raw)
        ));
    }

    #[test]
    fn known_text_predicate_is_case_insensitive() {
        assert!(Registry::is_known_text("LOG"));
        assert!(Registry::is_known_text("json"));
        assert!(!Registry::is_known_text("pdf"));
    }

    /// With a binary-format feature on, the default registry resolves that
    /// extension to a non-plain extractor.
    #[cfg(any(feature = "xls", feature = "docx", feature = "pdf"))]
    #[test]
    fn default_registry_resolves_enabled_formats() {
        let reg = default_registry();
        #[cfg(feature = "xls")]
        assert!(reg
            .resolve(Path::new("book.xlsx"))
            .extensions()
            .contains(&"xlsx"));
        #[cfg(feature = "docx")]
        assert!(reg
            .resolve(Path::new("memo.docx"))
            .extensions()
            .contains(&"docx"));
        #[cfg(feature = "pdf")]
        assert!(reg
            .resolve(Path::new("report.pdf"))
            .extensions()
            .contains(&"pdf"));
        // Plain text is never captured by a binary extractor.
        assert!(reg
            .resolve(Path::new("a.log"))
            .extensions()
            .contains(&"log"));
    }

    #[cfg(feature = "docx")]
    #[test]
    fn docx_xml_extracts_paragraph_text() {
        let xml = r#"<w:document xmlns:w="x"><w:body>
            <w:p><w:r><w:t>Hello</w:t><w:t> World</w:t></w:r></w:p>
            <w:p><w:r><w:t>second line</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let text = super::formats::docx_xml_to_text(xml);
        assert!(text.contains("Hello World"));
        assert!(text.contains("second line"));
        // Paragraphs are newline-separated.
        assert!(text.lines().count() >= 2);
    }

    #[cfg(feature = "docx")]
    #[test]
    fn docx_roundtrip_extract_and_scan() {
        use std::io::Write as _;
        let dir = std::env::temp_dir().join(format!("fetchy-docx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memo.docx");

        // A .docx is a zip; the extractor only reads word/document.xml.
        let f = std::fs::File::create(&path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        zw.start_file("word/document.xml", opts).unwrap();
        let doc = r#"<w:document xmlns:w="x"><w:body>
            <w:p><w:r><w:t>quarterly revenue report</w:t></w:r></w:p>
            <w:p><w:r><w:t>nothing to see</w:t></w:r></w:p>
        </w:body></w:document>"#;
        zw.write_all(doc.as_bytes()).unwrap();
        zw.finish().unwrap();

        // Direct extraction.
        let reg = default_registry();
        let src = reg.resolve(&path).extract(&path).unwrap();
        match src {
            Some(Source::Materialized(t)) => assert!(t.contains("quarterly revenue report")),
            other => panic!("expected materialized docx text, got {other:?}"),
        }

        // End-to-end scan finds the match inside the docx.
        let q = crate::model::Query::literal("revenue");
        let hits = crate::scan::search_collect(
            std::slice::from_ref(&dir),
            &q,
            crate::scan::ScanOptions::default(),
        );
        assert_eq!(hits.len(), 1, "hits: {hits:?}");
        assert!(hits[0].text.contains("revenue"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(feature = "xls")]
    #[test]
    fn xlsx_roundtrip_extract_and_scan() {
        use std::io::Write as _;
        let dir = std::env::temp_dir().join(format!("fetchy-xlsx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("book.xlsx");

        // Minimal but valid OOXML spreadsheet with inline strings (no
        // sharedStrings), enough for calamine to read one sheet.
        let parts: [(&str, &str); 5] = [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>Revenue</t></is></c><c r="B1" t="inlineStr"><is><t>Q3 summary</t></is></c></row>
<row r="2"><c r="A2" t="inlineStr"><is><t>other</t></is></c></row>
</sheetData>
</worksheet>"#,
            ),
        ];

        let f = std::fs::File::create(&path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, body) in parts {
            zw.start_file(name, opts).unwrap();
            zw.write_all(body.as_bytes()).unwrap();
        }
        zw.finish().unwrap();

        // Direct extraction yields the cell text (tab-separated per row).
        let reg = default_registry();
        match reg.resolve(&path).extract(&path).unwrap() {
            Some(Source::Materialized(t)) => {
                assert!(t.contains("Revenue"), "text: {t:?}");
                assert!(t.contains("Q3 summary"), "text: {t:?}");
            }
            other => panic!("expected materialized xlsx text, got {other:?}"),
        }

        // End-to-end scan finds a cell value inside the workbook.
        let q = crate::model::Query::literal("Q3 summary");
        let hits = crate::scan::search_collect(
            std::slice::from_ref(&dir),
            &q,
            crate::scan::ScanOptions::default(),
        );
        assert_eq!(hits.len(), 1, "hits: {hits:?}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
