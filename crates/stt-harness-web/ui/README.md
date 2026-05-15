# STT Harness review UI

React + Vite single-page app for reviewing diarized STT conversations
and editing per-conversation speaker labels. Vendors atomr-dashboard's
design system (HSL Tailwind tokens, card/badge/table primitives) so it
matches the dashboard in style.

## Develop

```bash
npm --prefix crates/stt-harness-web/ui ci
npm --prefix crates/stt-harness-web/ui run dev
```

The dev server runs on <http://localhost:5173> and proxies `/api` and
`/ws` to the axum backend (default bind `127.0.0.1:7000`). Start the
backend first — e.g. `cargo run -p atomr-agents-cli --features stt-web -- serve`.

## Production build

```bash
npm --prefix crates/stt-harness-web/ui run build      # writes ui/dist
cargo build -p atomr-agents-stt-harness-web --features embed-ui
```

`cargo build --features embed-ui` bakes `ui/dist/` into the binary via
`rust-embed`; `build.rs` fails fast if `ui/dist` is missing. The
canonical wrapper is `cargo xtask stt-web-build`.

`dist/` and `node_modules/` are git-ignored — the build output is
generated, never committed.

## Layout

- `src/lib/api.ts` — typed REST client + DTOs mirroring
  `atomr-agents-stt-harness`'s serde types.
- `src/lib/ws.ts` — reconnecting `/ws` event-stream hook.
- `src/components/ui/` — vendored design-system primitives.
- `src/components/conversation/` — `ConversationList`, `TranscriptView`,
  `SpeakerChip` (inline-editable label), `SpeakerLegend`, `LiveBadge`.
- `src/pages/` — the conversation list and the detail view.
