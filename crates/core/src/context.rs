use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::budget::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
use crate::ids::{AgentId, OrgId, TeamId};
use crate::value::Value;

/// Typed, tamper-evident extension map carried on [`CallCtx`].
///
/// This is the substrate-level channel through which a host attaches
/// caller identity / clearance / mandate (e.g. a `ClearanceContext`
/// from `atomr-agents-security`) so it flows unmodified to
/// `Tool::invoke` via [`InvokeCtx`]. Two deliberate guarantees:
///
/// * **Never persisted.** [`CallCtx`] is not `Serialize`, and the
///   extension map holds `dyn Any` values that cannot be serialized,
///   so secrets/clearance attached here never land in a checkpoint,
///   telemetry record, or prompt.
/// * **Not LLM-writable.** Values are inserted only by Rust host /
///   runtime code via [`Extensions::insert`] / [`CallCtx::insert_ext`];
///   there is no path from an LLM tool argument (`raw_args`) into this
///   map, so it is tamper-evident with respect to model output.
///
/// Stored values are `Arc`-wrapped so cloning a [`CallCtx`] (which the
/// runtime does per tool dispatch) shares — rather than deep-copies —
/// the extensions.
#[derive(Clone, Default)]
pub struct Extensions {
    map: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl Extensions {
    /// An empty extension map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert (or replace) the extension of type `T`.
    pub fn insert<T: Any + Send + Sync>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Arc::new(value));
    }

    /// Borrow the extension of type `T`, if present.
    pub fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|v| v.downcast_ref::<T>())
    }

    /// Whether an extension of type `T` is present.
    pub fn contains<T: Any + Send + Sync>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    /// Number of distinct extension types stored.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl std::fmt::Debug for Extensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Values are `dyn Any` and not `Debug`; report only the count so
        // CallCtx can still derive `Debug` without leaking contents.
        f.debug_struct("Extensions")
            .field("len", &self.map.len())
            .finish()
    }
}

/// Conversation message — mirrors `atomr_infer_core::batch::Message`
/// but lives at this layer so strategies can construct turns without
/// pulling in the full inference crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// What a single agent turn consumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnInput {
    pub user: String,
    #[serde(default)]
    pub history: Vec<Message>,
}

/// State the per-turn pipeline reads from. Strategies receive a
/// reference to this; they do not mutate it directly (they return
/// fragments which the `ContextAssembler` merges).
#[derive(Debug, Clone)]
pub struct AgentContext {
    pub agent_id: AgentId,
    pub team_id: Option<TeamId>,
    pub org_id: Option<OrgId>,
    pub turn: TurnInput,
}

impl AgentContext {
    pub fn for_agent(agent_id: AgentId, turn: TurnInput) -> Self {
        Self {
            agent_id,
            team_id: None,
            org_id: None,
            turn,
        }
    }
}

/// Context passed to `Callable::call`. Carries the budgets so a
/// callable can refuse work it can't afford, plus a typed
/// [`Extensions`] map for substrate-level caller context (clearance,
/// mandate, decision keys) that flows through to `Tool::invoke`.
#[derive(Debug, Clone)]
pub struct CallCtx {
    pub agent_id: Option<AgentId>,
    pub tokens: TokenBudget,
    pub time: TimeBudget,
    pub money: MoneyBudget,
    pub iterations: IterationBudget,
    pub trace: Vec<String>,
    /// Typed extension map (clearance, mandate, …). Never serialized,
    /// never LLM-writable. See [`Extensions`].
    pub extensions: Extensions,
}

impl CallCtx {
    /// Construct a [`CallCtx`] with an empty extension map.
    pub fn new(
        agent_id: Option<AgentId>,
        tokens: TokenBudget,
        time: TimeBudget,
        money: MoneyBudget,
        iterations: IterationBudget,
        trace: Vec<String>,
    ) -> Self {
        Self {
            agent_id,
            tokens,
            time,
            money,
            iterations,
            trace,
            extensions: Extensions::new(),
        }
    }

    /// Attach a typed extension, returning `self` for chaining.
    pub fn with_ext<T: Any + Send + Sync>(mut self, value: T) -> Self {
        self.extensions.insert(value);
        self
    }

    /// Attach a typed extension in place.
    pub fn insert_ext<T: Any + Send + Sync>(&mut self, value: T) {
        self.extensions.insert(value);
    }

    /// Borrow a typed extension of type `T`, if present.
    pub fn ext<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.extensions.get::<T>()
    }
}

/// Context passed to `Tool::invoke`.
#[derive(Debug, Clone)]
pub struct InvokeCtx {
    pub call: CallCtx,
    pub tool_call_id: String,
    pub raw_args: Value,
}

impl InvokeCtx {
    /// Borrow a typed extension propagated from the enclosing
    /// [`CallCtx`], if present.
    pub fn ext<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.call.ext::<T>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};

    #[derive(Debug, PartialEq)]
    struct Clearance(&'static str);

    fn ctx() -> CallCtx {
        CallCtx::new(
            None,
            TokenBudget::new(100),
            TimeBudget::new(std::time::Duration::from_secs(1)),
            MoneyBudget::from_usd(1.0),
            IterationBudget::new(1),
            vec![],
        )
    }

    #[test]
    fn extensions_roundtrip_by_type() {
        let mut c = ctx();
        assert!(c.ext::<Clearance>().is_none());
        c.insert_ext(Clearance("mnpi"));
        assert_eq!(c.ext::<Clearance>(), Some(&Clearance("mnpi")));
        assert!(c.extensions.contains::<Clearance>());
        assert_eq!(c.extensions.len(), 1);
    }

    #[test]
    fn extensions_survive_clone_and_propagate_to_invoke_ctx() {
        let c = ctx().with_ext(Clearance("internal"));
        let cloned = c.clone();
        assert_eq!(cloned.ext::<Clearance>(), Some(&Clearance("internal")));

        let ictx = InvokeCtx {
            call: c,
            tool_call_id: "t".into(),
            raw_args: Value::Null,
        };
        assert_eq!(ictx.ext::<Clearance>(), Some(&Clearance("internal")));
    }
}
