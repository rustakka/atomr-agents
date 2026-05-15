//! Fail-fast guard for the `embed-ui` feature.
//!
//! When the `embed-ui` feature is enabled, `rust-embed` bakes
//! `ui/dist` into the binary. If the React app has not been built,
//! the embed is silently empty and the server 404s on every asset.
//! This turns that into a clear build error. Build the SPA first:
//!
//! ```text
//! npm --prefix crates/meetings-harness-web/ui ci
//! npm --prefix crates/meetings-harness-web/ui run build
//! cargo build -p atomr-agents-meetings-harness-web --features embed-ui
//! ```

fn main() {
    println!("cargo:rerun-if-changed=ui/dist");

    if std::env::var_os("CARGO_FEATURE_EMBED_UI").is_some() {
        let index = std::path::Path::new("ui/dist/index.html");
        if !index.exists() {
            panic!(
                "feature `embed-ui` is enabled but `ui/dist/index.html` is missing.\n\
                 Build the React SPA first:\n  \
                 npm --prefix crates/meetings-harness-web/ui ci\n  \
                 npm --prefix crates/meetings-harness-web/ui run build"
            );
        }
    }
}
