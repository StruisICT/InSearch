fn main() {
    // On Windows, embed the manifest (DPI awareness, common controls v6,
    // supportedOS). No-op elsewhere. A failure here shouldn't break the build.
    #[cfg(windows)]
    {
        let _ = embed_resource::compile("app.rc", embed_resource::NONE);
    }
}
