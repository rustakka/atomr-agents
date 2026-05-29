//! FR-17 — debate / critique protocol primitive.
//!
//! Adversarial verification of investment theses is the core mechanism
//! for trustworthy research with real money: a proponent advances a
//! claim, a critic attacks it, and the loop runs until the critic raises
//! no unresolved dissent (consensus) or a round cap is hit. This module
//! provides that loop as a reusable primitive so each desk (research,
//! risk review, strategy review) supplies different personas/callables
//! instead of reimplementing a `ResearchConvergenceLoop` by hand.
//!
//! The full exchange is recorded into a [`CritiqueChannel`] — a typed
//! State channel built on [`atomr_agents_state::StateSchemaBuilder`] +
//! [`atomr_agents_state::AppendMessages`] — so the transcript composes
//! with the recording checkpointer and is replayable.
//!
//! Termination is a [`DissentTermination`] implementing
//! [`atomr_agents_strategy::TerminationStrategy`] over [`DebateState`],
//! surfacing a [`DebateOutcome`] with `converged`, the `unresolved`
//! dissents, and the ordered `transcript`.

use atomr_agents_callable::CallableHandle;
use atomr_agents_core::{CallCtx, Result, Value};
use atomr_agents_state::{AppendMessages, RunState, StateSchema};
use atomr_agents_strategy::{Termination, TerminationStrategy};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Who authored a turn and in what adversarial role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    Proponent,
    Critic,
    Judge,
}

/// One recorded turn of the debate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebateTurn {
    pub round: u32,
    pub author: String,
    pub stance: Stance,
    pub claim: String,
}

/// An unresolved objection raised by the critic in a given round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dissent {
    pub round: u32,
    pub critic: String,
    pub objection: String,
}

// ---------------------------------------------------------------------
// CritiqueChannel — typed State channel for the transcript.
// ---------------------------------------------------------------------

/// The state-schema channel key the transcript is appended to.
pub const CRITIQUE_CHANNEL: &str = "critique";

/// Helper around a [`StateSchema`] whose `critique` channel uses
/// [`AppendMessages`] to capture each `{ author, stance, claim }` turn.
///
/// Each appended message carries a stable `id` (`"{round}:{stance}"`) so
/// the `AppendMessages` reducer dedups re-emitted turns on replay.
pub struct CritiqueChannel;

impl CritiqueChannel {
    /// Build the state schema carrying the critique transcript channel.
    pub fn schema() -> StateSchema {
        StateSchema::builder().add(CRITIQUE_CHANNEL, AppendMessages).build()
    }

    /// A fresh [`RunState`] over [`CritiqueChannel::schema`].
    pub fn new_state() -> RunState {
        RunState::new(Arc::new(Self::schema()))
    }

    /// Append a turn into the critique channel of `state`.
    pub fn append(state: &mut RunState, turn: &DebateTurn) -> Result<()> {
        let id = format!("{}:{:?}", turn.round, turn.stance);
        let msg = serde_json::json!({
            "id": id,
            "round": turn.round,
            "author": turn.author,
            "stance": turn.stance,
            "claim": turn.claim,
        });
        state.write(CRITIQUE_CHANNEL, Value::Array(vec![msg]))
    }

    /// Read the ordered transcript back out of `state`.
    pub fn transcript(state: &RunState) -> Vec<DebateTurn> {
        match state.read(CRITIQUE_CHANNEL) {
            Value::Array(items) => items
                .iter()
                .filter_map(|m| serde_json::from_value::<DebateTurn>(m.clone()).ok())
                .collect(),
            _ => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------
// DebateState — what termination inspects.
// ---------------------------------------------------------------------

/// Accumulated debate progress. The [`DebateStrategy`] mutates this each
/// round; [`DissentTermination`] reads it to decide when to stop.
#[derive(Debug, Clone, Default)]
pub struct DebateState {
    pub round: u32,
    pub max_rounds: u32,
    /// Dissents raised in the most recent critic turn that are still open.
    pub unresolved: Vec<Dissent>,
    pub transcript: Vec<DebateTurn>,
    /// Set once the critic raised no dissent in a round.
    pub converged: bool,
}

// ---------------------------------------------------------------------
// Roles + strategy.
// ---------------------------------------------------------------------

/// The adversarial participants. `judge` is optional — when present it
/// renders a closing turn after the loop ends.
#[derive(Clone)]
pub struct DebateRoles {
    pub proponent: CallableHandle,
    pub critic: CallableHandle,
    pub judge: Option<CallableHandle>,
}

/// A structured adversarial debate. Alternates proponent → critic for up
/// to `max_rounds`, recording every turn into a [`CritiqueChannel`].
///
/// Callable contract (JSON):
/// * the **proponent** receives the running input and returns
///   `{ "claim": "<thesis or rebuttal>" }`;
/// * the **critic** receives the proponent's latest claim and returns
///   `{ "dissent": <bool>, "objection": "<why>" }` (a missing/false
///   `dissent` signals consensus for that round);
/// * the optional **judge** receives the final state and returns
///   `{ "claim": "<verdict>" }`.
pub struct DebateStrategy {
    pub roles: DebateRoles,
    pub max_rounds: u32,
}

impl DebateStrategy {
    pub fn new(roles: DebateRoles, max_rounds: u32) -> Self {
        Self { roles, max_rounds }
    }

    fn author_of(handle: &CallableHandle) -> String {
        handle.label().to_string()
    }

    /// Run the debate to convergence or `max_rounds`. Returns the
    /// [`DebateOutcome`] and leaves the recorded transcript in the
    /// returned [`RunState`] (the [`CritiqueChannel`]).
    pub async fn run(&self, initial: Value, ctx: CallCtx) -> Result<(DebateOutcome, RunState)> {
        let mut state = CritiqueChannel::new_state();
        let mut debate = DebateState {
            max_rounds: self.max_rounds,
            ..Default::default()
        };
        let termination = DissentTermination;

        let mut current = initial;
        for round in 1..=self.max_rounds {
            debate.round = round;

            // Proponent advances a claim / rebuttal.
            let prop_out = self.roles.proponent.call(current.clone(), ctx.clone()).await?;
            let claim = prop_out
                .get("claim")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let prop_turn = DebateTurn {
                round,
                author: Self::author_of(&self.roles.proponent),
                stance: Stance::Proponent,
                claim: claim.clone(),
            };
            CritiqueChannel::append(&mut state, &prop_turn)?;
            debate.transcript.push(prop_turn);

            // Critic attacks the claim.
            let critic_in = serde_json::json!({ "claim": claim });
            let critic_out = self.roles.critic.call(critic_in, ctx.clone()).await?;
            let dissents = critic_out
                .get("dissent")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let objection = critic_out
                .get("objection")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let critic_turn = DebateTurn {
                round,
                author: Self::author_of(&self.roles.critic),
                stance: Stance::Critic,
                claim: objection.clone(),
            };
            CritiqueChannel::append(&mut state, &critic_turn)?;
            debate.transcript.push(critic_turn);

            if dissents {
                debate.unresolved = vec![Dissent {
                    round,
                    critic: Self::author_of(&self.roles.critic),
                    objection,
                }];
            } else {
                debate.unresolved.clear();
                debate.converged = true;
            }

            // Feed the critic's objection forward so the proponent can
            // rebut next round.
            current = critic_out;

            if termination.should_terminate(&debate) != Termination::Continue {
                break;
            }
        }

        // Optional judge renders a closing verdict.
        if let Some(judge) = &self.roles.judge {
            let verdict_out = judge.call(current.clone(), ctx.clone()).await?;
            let verdict = verdict_out
                .get("claim")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let judge_turn = DebateTurn {
                round: debate.round,
                author: Self::author_of(judge),
                stance: Stance::Judge,
                claim: verdict,
            };
            CritiqueChannel::append(&mut state, &judge_turn)?;
            debate.transcript.push(judge_turn);
        }

        let outcome = DebateOutcome {
            converged: debate.converged,
            unresolved: debate.unresolved.clone(),
            transcript: debate.transcript.clone(),
        };
        Ok((outcome, state))
    }
}

/// Result of a debate. `converged` is true when the critic raised no
/// unresolved dissent before the round cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebateOutcome {
    pub converged: bool,
    pub unresolved: Vec<Dissent>,
    pub transcript: Vec<DebateTurn>,
}

// ---------------------------------------------------------------------
// DissentTermination.
// ---------------------------------------------------------------------

/// Ends the debate when consensus is reached (no unresolved dissent) or
/// the round cap is hit.
pub struct DissentTermination;

impl TerminationStrategy<DebateState> for DissentTermination {
    fn should_terminate(&self, state: &DebateState) -> Termination {
        if state.converged && state.unresolved.is_empty() {
            return Termination::Done("consensus");
        }
        if state.round >= state.max_rounds {
            return Termination::Done("max_rounds");
        }
        Termination::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_callable::FnCallable;
    use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
    use std::time::Duration;

    fn ctx() -> CallCtx {
        CallCtx {
            agent_id: None,
            tokens: TokenBudget::new(10_000),
            time: TimeBudget::new(Duration::from_secs(30)),
            money: MoneyBudget::from_usd(1.0),
            iterations: IterationBudget::new(32),
            trace: vec![],
            extensions: Default::default(),
        }
    }

    fn proponent() -> CallableHandle {
        Arc::new(FnCallable::labeled("proponent", |_v: Value, _ctx| async move {
            Ok(serde_json::json!({ "claim": "Buy ACME: undervalued vs peers." }))
        }))
    }

    /// A critic that never dissents → immediate consensus.
    fn agreeable_critic() -> CallableHandle {
        Arc::new(FnCallable::labeled("critic", |_v: Value, _ctx| async move {
            Ok(serde_json::json!({ "dissent": false, "objection": "" }))
        }))
    }

    /// A critic that always dissents → never converges.
    fn relentless_critic() -> CallableHandle {
        Arc::new(FnCallable::labeled("critic", |_v: Value, _ctx| async move {
            Ok(serde_json::json!({ "dissent": true, "objection": "Liquidity risk understated." }))
        }))
    }

    #[tokio::test]
    async fn converges_when_critic_raises_no_dissent() {
        let strategy = DebateStrategy::new(
            DebateRoles {
                proponent: proponent(),
                critic: agreeable_critic(),
                judge: None,
            },
            5,
        );
        let (outcome, state) = strategy.run(serde_json::json!({}), ctx()).await.unwrap();
        assert!(outcome.converged);
        assert!(outcome.unresolved.is_empty());
        // One round: proponent + critic = 2 turns.
        assert_eq!(outcome.transcript.len(), 2);
        // Transcript is recorded into the critique channel in order.
        let recorded = CritiqueChannel::transcript(&state);
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].stance, Stance::Proponent);
        assert_eq!(recorded[1].stance, Stance::Critic);
    }

    #[tokio::test]
    async fn relentless_dissent_hits_max_rounds_with_unresolved() {
        let max = 3;
        let strategy = DebateStrategy::new(
            DebateRoles {
                proponent: proponent(),
                critic: relentless_critic(),
                judge: None,
            },
            max,
        );
        let (outcome, _state) = strategy.run(serde_json::json!({}), ctx()).await.unwrap();
        assert!(!outcome.converged);
        assert!(!outcome.unresolved.is_empty());
        // 3 rounds × (proponent + critic) = 6 turns.
        assert_eq!(outcome.transcript.len(), (max * 2) as usize);
        assert_eq!(outcome.unresolved[0].round, max);
    }

    #[tokio::test]
    async fn judge_appends_closing_turn() {
        let judge: CallableHandle = Arc::new(FnCallable::labeled("judge", |_v: Value, _ctx| async move {
            Ok(serde_json::json!({ "claim": "Verdict: proceed with reduced size." }))
        }));
        let strategy = DebateStrategy::new(
            DebateRoles {
                proponent: proponent(),
                critic: agreeable_critic(),
                judge: Some(judge),
            },
            2,
        );
        let (outcome, _state) = strategy.run(serde_json::json!({}), ctx()).await.unwrap();
        let last = outcome.transcript.last().unwrap();
        assert_eq!(last.stance, Stance::Judge);
        assert!(last.claim.contains("Verdict"));
    }

    #[test]
    fn termination_reports_consensus_and_cap() {
        let t = DissentTermination;
        let mut s = DebateState {
            round: 1,
            max_rounds: 5,
            converged: true,
            ..Default::default()
        };
        assert_eq!(t.should_terminate(&s), Termination::Done("consensus"));

        s.converged = false;
        s.unresolved = vec![Dissent {
            round: 5,
            critic: "c".into(),
            objection: "x".into(),
        }];
        s.round = 5;
        assert_eq!(t.should_terminate(&s), Termination::Done("max_rounds"));

        s.round = 2;
        assert_eq!(t.should_terminate(&s), Termination::Continue);
    }
}
