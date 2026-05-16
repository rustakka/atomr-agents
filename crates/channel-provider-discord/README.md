# atomr-agents-channel-provider-discord

Discord [`ChannelProvider`] for the atomr-agents channel layer.

Two flavors, selected per channel:

- **Gateway mode** — `start` opens a WS to `wss://gateway.discord.gg/`, handles
  identify + heartbeats, and emits inbound `MESSAGE_CREATE` events.
- **Interactions webhook mode** — `start` is a no-op; `verify_webhook` does
  Ed25519 over `timestamp || body` against the application's public key;
  `parse_webhook` maps the interaction payload.

Outbound goes through the REST endpoint
`POST /channels/{channel_id}/messages`. `fetch_media` retrieves attachment URLs
directly (Discord serves them).

Configuration is parsed from `serde_json::Value` so the harness can attach a
provider built from any source (env vars, REST body, Python dict). See
`config.rs` for the supported keys.

The channel layer is purely additive: agents and harnesses still expose
`turn()` / `run()` unchanged — this provider simply lets a Discord message
become a thread that invokes any [`Callable`].

[`ChannelProvider`]: ../channel-core/src/provider.rs
[`Callable`]: ../callable/src/lib.rs
