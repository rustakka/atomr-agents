# atomr-agents-channel-core

Domain types for the channels-and-threads layer.

A **channel** is a provider-specific messaging transport
(WhatsApp / Signal / Discord / in-memory). A **thread** is a long-lived
conversation between a channel peer and a bound
[`Callable`](../callable/) target (agent, harness, team, workflow, …).

Channels are an **optional, additive** layer over the existing agent
conversation surface. They don't change `AgentRef::turn` or
`HarnessRef::run`; they just expose the same callables behind a
messaging API.

## What this crate ships

| Item                  | Purpose                                                                |
|-----------------------|------------------------------------------------------------------------|
| `ChannelId` / `ThreadId` / `PeerId` | Newtype ids matching the repo convention.                |
| `ChannelSpec`, `Capabilities`, `ProviderKind` | Persisted description of one channel.            |
| `InboundMessage`, `OutboundMessage`, `ProviderAck`, `ChannelMessageRecord` | Wire types. |
| `ChannelProvider` trait | What every transport implements (`start`, `send`, `verify_webhook`, `parse_webhook`, `fetch_media`). |
| `ProviderHandle`        | Cooperative shutdown handle (matches `stt-harness::SessionHandle`).  |
| `ThreadTarget`, `HarnessInputAdapter` | Bind target — either a raw `Callable` or a `HarnessRef` mediated by an adapter (because `HarnessRef::call` ignores its input). |
| `Thread`, `ThreadRef`, `ThreadPolicy` | Per-conversation state. `ThreadRef` implements `Callable`. |
| `ChannelStore`, `InMemoryChannelStore`  | Persistence surface — same shape as `MeetingsStore`. |
| `ChannelEvent`, `ChannelEventStream`    | `tokio::broadcast`-based event stream the `-web` companion bridges to a WebSocket. |
| `memory::InMemoryProvider`              | In-process provider used by every integration test.        |

## Why an explicit `ThreadTarget`

`HarnessRef::call` drops its `Value` input and just calls `run()`
(see `crates/harness/src/dispatch.rs`). If we naively bound a harness
to a thread via `Callable`, every inbound message would be silently
ignored. The `ThreadTarget::Harness { adapter, … }` variant fixes this:
the orchestrator routes the inbound through the adapter (which can,
e.g., append a turn to the harness's source store) before running the
harness — so the harness sees the inbound it was meant to react to.

For everything else — `AgentRef`, teams, workflow steps, plain
`FnCallable`s — `ThreadTarget::Callable` is enough, because those
already accept an input envelope.

## Provider crates

Real transports live in their own crates so each can pull its own SDK:

- [`atomr-agents-channel-provider-whatsapp`](../channel-provider-whatsapp/) — WhatsApp Business Cloud API (HMAC-SHA256 webhook)
- [`atomr-agents-channel-provider-signal`](../channel-provider-signal/) — `signal-cli` JSON-RPC bridge
- [`atomr-agents-channel-provider-discord`](../channel-provider-discord/) — Discord Gateway WS / Interactions webhook (Ed25519)

## Feature flags

| Feature  | Effect                                                                |
|----------|-----------------------------------------------------------------------|
| (none, default) | In-memory store only.                                          |
| `state`  | Adds a checkpointer-backed store wired through `atomr-agents-state`. |
