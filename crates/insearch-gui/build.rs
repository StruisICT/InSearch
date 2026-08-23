fn main() {
    // Expose the real release version (release-please tracks it in the manifest;
    // it doesn't sync the workspace Cargo.toml, so CARGO_PKG_VERSION is stale).
    // Falls back to CARGO_PKG_VERSION if the manifest can't be read.
    let version = manifest_version().unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=INSEARCH_VERSION={version}");
    println!("cargo:rerun-if-changed=../../.release-please-manifest.json");

    // On Windows, embed the manifest (DPI awareness, common controls v6,
    // supportedOS). No-op elsewhere. A failure here shouldn't break the build.
    #[cfg(windows)]
    {
        let _ = embed_resource::compile("app.rc", embed_resource::NONE);
    }
}

/// Read the release version from `.release-please-manifest.json` (`{ ".": "x" }`)
/// at the workspace root, without pulling in a JSON dependency.
fn manifest_version() -> Option<String> {
    let dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let path = std::path::Path::new(&dir)
        .join("..")
        .join("..")
        .join(".release-please-manifest.json");
    let text = std::fs::read_to_string(path).ok()?;
    // Grab the string value that follows the "." key.
    let after_key = text.split("\".\"").nth(1)?;
    let after_colon = &after_key[after_key.find(':')? + 1..];
    let start = after_colon.find('"')? + 1;
    let end = after_colon[start..].find('"')? + start;
    Some(after_colon[start..end].to_string())
}
