use std::sync::Arc;

use atomr_agents_core::ToolSetId;
use dashmap::DashMap;
use semver::Version;

use crate::toolset::ToolSet;

/// Thread-safe registry of versioned tool sets. Multiple versions of
/// the same id can coexist; consumers pin the version they want.
#[derive(Default, Clone)]
pub struct ToolSetRegistry {
    inner: Arc<DashMap<(ToolSetId, Version), Arc<ToolSet>>>,
}

impl ToolSetRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn publish(&self, ts: ToolSet) -> Arc<ToolSet> {
        let key = (ts.id.clone(), ts.version.clone());
        let arc = Arc::new(ts);
        self.inner.insert(key, arc.clone());
        arc
    }

    pub fn get(&self, id: &ToolSetId, version: &Version) -> Option<Arc<ToolSet>> {
        self.inner.get(&(id.clone(), version.clone())).map(|r| r.value().clone())
    }

    /// Highest version of `id`, or `None` if none published.
    pub fn latest(&self, id: &ToolSetId) -> Option<Arc<ToolSet>> {
        self.inner
            .iter()
            .filter(|r| r.key().0.as_str() == id.as_str())
            .map(|r| r.value().clone())
            .max_by(|a, b| a.version.cmp(&b.version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{ToolDescriptor, ToolSchema};
    use crate::r#trait::{DynTool, Tool};
    use async_trait::async_trait;
    use atomr_agents_core::{InvokeCtx, Result, ToolId, Value};

    struct EchoTool {
        descriptor: ToolDescriptor,
    }
    impl EchoTool {
        fn new() -> Self {
            Self {
                descriptor: ToolDescriptor {
                    id: ToolId::from("echo"),
                    name: "echo".into(),
                    description: "echo input".into(),
                    schema: ToolSchema::empty_object(),
                },
            }
        }
    }
    #[async_trait]
    impl Tool for EchoTool {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.descriptor
        }
        async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> Result<Value> {
            Ok(args)
        }
    }

    #[test]
    fn publish_get_latest() {
        let r = ToolSetRegistry::new();
        let tools: Vec<DynTool> = vec![Arc::new(EchoTool::new())];
        let ts1 = ToolSet::new("echos", Version::new(0, 1, 0), tools.clone());
        let ts2 = ToolSet::new("echos", Version::new(0, 2, 0), tools);
        r.publish(ts1);
        r.publish(ts2);
        let latest = r.latest(&ToolSetId::from("echos")).unwrap();
        assert_eq!(latest.version, Version::new(0, 2, 0));
        let pinned = r.get(&ToolSetId::from("echos"), &Version::new(0, 1, 0)).unwrap();
        assert_eq!(pinned.version, Version::new(0, 1, 0));
    }
}
