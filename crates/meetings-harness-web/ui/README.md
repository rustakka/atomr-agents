# meetings-harness-web — UI

React + Vite SPA for the meetings harness. Mirrors the STT review UI
(`crates/stt-harness-web/ui`); shares the same design tokens.

## Dev

```bash
npm --prefix crates/meetings-harness-web/ui ci
npm --prefix crates/meetings-harness-web/ui run dev   # SPA on :5174

# In another terminal, run the axum server:
cargo run -p atomr-agents-cli -- meetings serve --bind 127.0.0.1:7100
```

The Vite dev server proxies `/api` and `/ws` to `:7100`.

## Production (embedded)

```bash
npm --prefix crates/meetings-harness-web/ui run build
cargo build -p atomr-agents-meetings-harness-web --features embed-ui
```
