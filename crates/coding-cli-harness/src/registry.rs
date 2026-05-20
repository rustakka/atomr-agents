//! Registry mapping `CliVendorKind` → adapter.

use std::collections::HashMap;
use std::sync::Arc;

use atomr_agents_coding_cli_core::{CliVendor, CliVendorKind};

#[derive(Default, Clone)]
pub struct VendorRegistry {
    inner: HashMap<CliVendorKind, Arc<dyn CliVendor>>,
}

impl VendorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, vendor: Arc<dyn CliVendor>) -> Self {
        self.inner.insert(vendor.kind(), vendor);
        self
    }

    pub fn insert(&mut self, vendor: Arc<dyn CliVendor>) {
        self.inner.insert(vendor.kind(), vendor);
    }

    pub fn get(&self, kind: &CliVendorKind) -> Option<Arc<dyn CliVendor>> {
        self.inner.get(kind).cloned()
    }

    pub fn kinds(&self) -> impl Iterator<Item = &CliVendorKind> {
        self.inner.keys()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&CliVendorKind, &Arc<dyn CliVendor>)> {
        self.inner.iter()
    }

    /// Build a registry with the three default vendor adapters wired
    /// up. Each is gated by a Cargo feature; when a feature is off
    /// the corresponding adapter is silently omitted.
    pub fn default_vendors() -> Self {
        let mut r = Self::new();
        #[cfg(feature = "vendor-claude")]
        {
            r.insert(Arc::new(
                atomr_agents_coding_cli_vendor_claude::ClaudeVendor::new(),
            ));
        }
        #[cfg(feature = "vendor-codex")]
        {
            r.insert(Arc::new(
                atomr_agents_coding_cli_vendor_codex::CodexVendor::new(),
            ));
        }
        #[cfg(feature = "vendor-antigravity")]
        {
            r.insert(Arc::new(
                atomr_agents_coding_cli_vendor_antigravity::AntigravityVendor::new(),
            ));
        }
        r
    }
}
