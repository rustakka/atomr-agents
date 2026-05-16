# atomr-agents-channel-harness

The channel orchestrator. Wraps a set of attached [`ChannelProvider`]s in a
single runtime that:

- Routes inbound messages from every provider through a dedup gate
  (`(channel_id, provider_msg_id)`), opens or looks up a [`Thread`] per peer,
  and dispatches to the bound [`ThreadTarget`].
- Runs **one outbound worker per channel**, draining a per-channel queue with
  bounded retry, idempotency by caller-supplied key, and capability checks.
- Emits a [`ChannelEvent`] broadcast stream (per-channel lifecycle, every
  received / sent message, every turn) consumed by the optional `-web`
  companion and Python observers.
- Persists channels, threads, and messages through a pluggable
  [`ChannelStore`] (the default in-memory store is fine for tests; a
  checkpointer-backed store is feature-gated behind `state`).

## Example

```rust,no_run
use std::sync::Arc;
use atomr_agents_callable::FnCallable;
use atomr_agents_channel_core::memory::InMemoryProvider;
use atomr_agents_channel_core::{
    ChannelId, ChannelSpec, PeerId, ProviderKind, ThreadTarget,
};
use atomr_agents_channel_harness::ChannelHarness;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let harness = ChannelHarness::in_memory();
let channel = ChannelId::from("memory:dev");
let provider = Arc::new(InMemoryProvider::new(channel.clone()));
harness
    .attach_provider(ChannelSpec::new(channel.clone(), ProviderKind::Memory), provider)
    .await?;

let echo: atomr_agents_callable::CallableHandle = Arc::new(FnCallable::labeled(
    "echo",
    |v: atomr_agents_core::Value, _ctx| async move { Ok(v) },
));
harness
    .open_thread(&channel, PeerId::from("alice"), ThreadTarget::callable(echo))
    .await?;
# Ok(()) }
```

## Features

- `default = ["providers-all"]`
- `provider-whatsapp` / `provider-signal` / `provider-discord` — wire the
  per-provider crate in via the harness's transitive deps.
- `state` — checkpointer-backed `ChannelStore` for persisted runs.

[`ChannelProvider`]: ../channel-core/src/provider.rs
[`Thread`]: ../channel-core/src/thread.rs
[`ThreadTarget`]: ../channel-core/src/target.rs
[`ChannelEvent`]: ../channel-core/src/events.rs
[`ChannelStore`]: ../channel-core/src/store.rs
