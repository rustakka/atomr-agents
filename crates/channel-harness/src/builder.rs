use std::sync::Arc;

use atomr_agents_channel_core::{ChannelStore, InMemoryChannelStore, ThreadPolicy, ThreadTarget};
use atomr_agents_registry::Registry;

use crate::harness::ChannelHarness;

/// Builder for [`ChannelHarness`].
#[derive(Default)]
pub struct ChannelHarnessBuilder {
    store: Option<Arc<dyn ChannelStore>>,
    registry: Option<Arc<Registry>>,
    default_policy: ThreadPolicy,
    auto_open_target: Option<ThreadTarget>,
}

impl ChannelHarnessBuilder {
    pub fn with_store(mut self, store: Arc<dyn ChannelStore>) -> Self {
        self.store = Some(store);
        self
    }

    pub fn with_registry(mut self, registry: Arc<Registry>) -> Self {
        self.registry = Some(registry);
        self
    }

    pub fn with_default_policy(mut self, policy: ThreadPolicy) -> Self {
        self.default_policy = policy;
        self
    }

    /// If set, inbound messages from unknown peers auto-open a thread
    /// bound to this target. If unset, unknown inbound is dropped with
    /// an `Error` event.
    pub fn with_auto_open_target(mut self, target: ThreadTarget) -> Self {
        self.auto_open_target = Some(target);
        self
    }

    pub fn build(self) -> ChannelHarness {
        let store = self.store.unwrap_or_else(|| Arc::new(InMemoryChannelStore::new()));
        ChannelHarness::from_parts(store, self.registry, self.default_policy, self.auto_open_target)
    }
}
