//! The two-tier outer shell — routes a [`ResearchRequest`] to either
//! a [`ShallowResearcher`] or a [`DeepResearchHarnessRef`] based on the
//! verdict of an [`IntentClassifier`].

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, Result as CoreResult, Value};
use atomr_agents_deep_research_core::{ResearchRequest, ResearchResult};
use atomr_agents_deep_research_harness::DeepResearchHarnessRef;

use crate::classifier::{IntentClassifier, ResearchTier};
use crate::error::ShellError;
use crate::shallow::ShallowResearcher;

/// Two-tier outer shell.
///
/// Holds an [`IntentClassifier`], a [`ShallowResearcher`], and a
/// [`DeepResearchHarnessRef`]; implements [`Callable`] so it composes
/// like any other tool / harness in the framework.
#[derive(Clone)]
pub struct DeepResearchShell {
    classifier: Arc<dyn IntentClassifier>,
    shallow: Arc<dyn ShallowResearcher>,
    deep: DeepResearchHarnessRef,
    label: String,
}

impl DeepResearchShell {
    /// Wire a shell around an intent classifier, a shallow researcher,
    /// and a deep-research harness handle.
    pub fn new(
        classifier: Arc<dyn IntentClassifier>,
        shallow: Arc<dyn ShallowResearcher>,
        deep: DeepResearchHarnessRef,
    ) -> Self {
        let label = format!("deep-research-shell:{}", deep.id.as_str());
        Self {
            classifier,
            shallow,
            deep,
            label,
        }
    }

    /// Access the underlying deep-research harness handle.
    pub fn deep(&self) -> &DeepResearchHarnessRef {
        &self.deep
    }

    /// Pure async run path: classify then dispatch to shallow or deep.
    pub async fn run(&self, req: ResearchRequest) -> CoreResult<ResearchResult> {
        let tier = self.classifier.classify(&req).await.map_err(|e| {
            // Preserve the original error if it's already a shell error,
            // otherwise wrap as a classifier error.
            match e {
                ShellError::Classifier(_) => e,
                other => ShellError::Classifier(other.to_string()),
            }
        })?;
        match tier {
            ResearchTier::Shallow => Ok(self.shallow.run(&req).await?),
            ResearchTier::Deep => {
                let v = self.deep.run(req).await?;
                Ok(serde_json::from_value::<ResearchResult>(v).map_err(ShellError::Serde)?)
            }
        }
    }
}

#[async_trait]
impl Callable for DeepResearchShell {
    async fn call(&self, input: Value, _ctx: CallCtx) -> CoreResult<Value> {
        let req = parse_request(input)?;
        let result = self.run(req).await?;
        Ok(serde_json::to_value(&result).map_err(ShellError::Serde)?)
    }

    fn label(&self) -> &str {
        &self.label
    }
}

/// Parse a JSON `Value` into a [`ResearchRequest`]. Mirrors
/// `atomr_agents_deep_research_harness::parse_request` — a bare string
/// is shorthand for `{"query": "..."}`.
fn parse_request(input: Value) -> CoreResult<ResearchRequest> {
    if let Some(s) = input.as_str() {
        return Ok(ResearchRequest::new(s));
    }
    serde_json::from_value(input).map_err(|e| ShellError::Serde(e).into())
}
