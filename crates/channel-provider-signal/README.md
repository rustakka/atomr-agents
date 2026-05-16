# atomr-agents-channel-provider-signal

Signal provider for the atomr-agents [channels feature][channel-core]. Drives a
local [`signal-cli`][signal-cli] daemon over its line-delimited JSON-RPC 2.0
interface — either a Unix domain socket or a TCP listener.

## signal-cli setup

This crate assumes you've already registered or linked a Signal account
through `signal-cli` and have a daemon running. Two transport modes are
supported:

```text
# TCP (handy on dev machines, no socket permissions to manage):
signal-cli --output=json daemon --tcp 127.0.0.1:7583

# Unix socket (preferred for prod — file-system permissions gate access):
signal-cli --output=json daemon --socket /tmp/signal-cli.sock
```

Multi-account daemons are supported. The provider always sends the configured
`account` as `params.account` on every JSON-RPC `send` call.

## Configuration

The provider reads the `config` blob on a `ChannelSpec`. Two endpoint shapes
are accepted:

```json
{
  "endpoint": { "transport": "tcp", "address": "127.0.0.1:7583" },
  "account": "+15551234567",
  "default_channel_id": "channel-signal-demo"
}
```

```json
{
  "endpoint": { "transport": "unix", "address": "/tmp/signal-cli.sock" },
  "account": "+15551234567",
  "default_channel_id": "channel-signal-demo"
}
```

Fields:

- `endpoint.transport` — `"tcp"` or `"unix"`.
- `endpoint.address` — `host:port` for TCP or filesystem path for Unix.
- `account` — the Signal phone number this provider sends as (E.164).
- `default_channel_id` — channel id assigned to all inbound events from this
  socket. signal-cli notifications carry no channel context of their own, so
  the provider stamps everything with this id.

## Capabilities

| Capability | Supported |
|------------|-----------|
| text       | yes       |
| attachments| yes       |
| voice      | no        |
| reactions  | no        |
| typing     | no        |
| threads    | no (synthesized from `channel#peer`) |

Attachment outbound: the provider treats `MessageContent::Attachment::media_ref`
as a **local filesystem path** that `signal-cli` itself can read — that's the
contract signal-cli expects on `params.attachments`.

Attachment inbound: signal-cli puts the file id (and content type) on each
`attachments[]` entry. The provider keeps the id in `media_ref`. Resolving
attachment bytes via `fetch_media(id)` reads the file from disk; in practice
you'll point `media_ref` at the path signal-cli stored the file under, since
the bare attachment id isn't a path on its own.

## Example

```rust,no_run
use atomr_agents_channel_provider_signal::SignalProvider;
use atomr_agents_channel_core::{ChannelProvider, OutboundMessage, MessageContent};
use serde_json::json;
use tokio::sync::mpsc;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let provider = SignalProvider::from_value(json!({
    "endpoint": { "transport": "tcp", "address": "127.0.0.1:7583" },
    "account": "+15551234567",
    "default_channel_id": "channel-signal-demo"
}))?;

let (in_tx, mut in_rx) = mpsc::channel(64);
let handle = provider.start(in_tx).await?;

provider.send(OutboundMessage {
    channel_id: "channel-signal-demo".into(),
    thread_id: "t1".into(),
    peer: "+15559876543".into(),
    content: MessageContent::text("hello from atomr-agents"),
    reply_to: None,
    idempotency_key: "first-hello".into(),
}).await?;

while let Some(msg) = in_rx.recv().await {
    println!("inbound: {}", msg.content.summary());
    break;
}

handle.signal_stop();
handle.join.await.ok();
# Ok(()) }
```

## Implementation notes

- Single reader task owns the read half of the socket and demultiplexes
  responses (matched by JSON-RPC `id`) and notifications (`method == "receive"`).
- Single writer task drains an mpsc queue of pre-serialized frames so concurrent
  `send()` callers don't contend on the socket directly.
- `verify_webhook` / `parse_webhook` return `Unsupported` — signal-cli is not
  webhook-driven.

[channel-core]: ../channel-core/README.md
[signal-cli]: https://github.com/AsamK/signal-cli
