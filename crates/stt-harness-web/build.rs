//! Fail-fast guard for the `embed-ui` feature.
//!
//! `rust-embed` bakes `ui/dist` into the binary at compile time; if the
//! React app has not been built yet, the embed is silently empty and
//! the server 404s on every asset. This check turns that into a clear
//! build error instead. Build the SPA first:
//!
//! ```text
//! npm --prefix crates/stt-harness-web/ui ci
//! npm --prefix crates/stt-harness-web/ui run build
//! cargo build -p atomr-agents-stt-harness-web --features embed-ui
//! ```

fn main() {
    println!("cargo:rerun-if-changed=ui/dist");

    if std::env::var_os("CARGO_FEATURE_EMBED_UI").is_some() {
        let index = std::path::Path::new("ui/dist/index.html");
        if !index.exists() {
            panic!(
                "feature `embed-ui` is enabled but `ui/dist/index.html` is missing.\n\
                 Build the React SPA first:\n  \
                 npm --prefix crates/stt-harness-web/ui ci\n  \
                 npm --prefix crates/stt-harness-web/ui run build"
            );
        }
    }
}
