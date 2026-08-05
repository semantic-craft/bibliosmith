fn main() {
    // A clean checkout intentionally contains no generated runtime bundle or
    // sidecar binaries. Debug-only Cargo commands do not package an App, so
    // keep `cargo check`, Clippy, and tests independent of release artifacts.
    // Release builds retain the checked-in bundle contract and fail if the
    // preparation step did not materialize the pinned resources first.
    if tauri_build::is_dev() && std::env::var_os("TAURI_CONFIG").is_none() {
        std::env::set_var(
            "TAURI_CONFIG",
            r#"{"bundle":{"externalBin":[],"resources":[]}}"#,
        );
    }
    tauri_build::build();
}
