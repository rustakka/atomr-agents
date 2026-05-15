//! Termination strategies for the STT harness loop.
//!
//! Checked at the top of every iteration, mirroring
//! `atomr_agents_harness::TerminationStrategy`. Stream-end is handled
//! by the loop strategy returning `Done`; these strategies impose
//! *additional* caps (utterance count, audio seconds, budget).

use crate::state::SttHarnessState;

/// Outcome of a termination check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Termination {
    /// Keep looping.
    Continue,
    /// Stop now; the `&'static str` is the reason, surfaced in
    /// telemetry as `terminated:<reason>`.
    Done(&'static str),
}

/// Decides whether the harness loop should stop.
pub trait SttTermination: Send + Sync + 'static {
    fn should_terminate(&self, state: &SttHarnessState) -> Termination;
}

impl SttTermination for Box<dyn SttTermination> {
    fn should_terminate(&self, state: &SttHarnessState) -> Termination {
        (**self).should_terminate(state)
    }
}

/// Run until the audio stream ends (the loop strategy returns `Done`).
/// Imposes no extra cap — this is the default.
#[derive(Debug, Default, Clone, Copy)]
pub struct StreamEndTermination;

impl SttTermination for StreamEndTermination {
    fn should_terminate(&self, _state: &SttHarnessState) -> Termination {
        Termination::Continue
    }
}

/// Stop once `cap` utterances have been committed.
#[derive(Debug, Clone, Copy)]
pub struct UtteranceCapTermination {
    pub cap: usize,
}

impl SttTermination for UtteranceCapTermination {
    fn should_terminate(&self, state: &SttHarnessState) -> Termination {
        if state.conversation.turns.len() >= self.cap {
            Termination::Done("utterance_cap")
        } else {
            Termination::Continue
        }
    }
}

/// Stop once the conversation covers at least `max_secs` of audio.
#[derive(Debug, Clone, Copy)]
pub struct AudioSecsTermination {
    pub max_secs: f32,
}

impl SttTermination for AudioSecsTermination {
    fn should_terminate(&self, state: &SttHarnessState) -> Termination {
        if state.conversation.total_audio_secs >= self.max_secs {
            Termination::Done("audio_secs")
        } else {
            Termination::Continue
        }
    }
}

/// Stop once the token-shaped budget proxy is exhausted.
#[derive(Debug, Default, Clone, Copy)]
pub struct BudgetTermination;

impl SttTermination for BudgetTermination {
    fn should_terminate(&self, state: &SttHarnessState) -> Termination {
        if state.remaining_budget == 0 {
            Termination::Done("budget_exhausted")
        } else {
            Termination::Continue
        }
    }
}

/// OR-combine several strategies: the first to fire wins.
pub struct CompositeTermination(pub Vec<Box<dyn SttTermination>>);

impl CompositeTermination {
    pub fn new(strategies: Vec<Box<dyn SttTermination>>) -> Self {
        Self(strategies)
    }
}

impl SttTermination for CompositeTermination {
    fn should_terminate(&self, state: &SttHarnessState) -> Termination {
        for strat in &self.0 {
            if let Termination::Done(reason) = strat.should_terminate(state) {
                return Termination::Done(reason);
            }
        }
        Termination::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::SttConversation;

    fn state_with(turns: usize, audio_secs: f32, budget: u32) -> SttHarnessState {
        let mut s = SttHarnessState::new("c1", budget);
        let mut conv = SttConversation::new("c1");
        for _ in 0..turns {
            conv.append_agent_reply("x");
        }
        conv.total_audio_secs = audio_secs;
        s.conversation = conv;
        s
    }

    #[test]
    fn stream_end_never_fires_on_its_own() {
        let s = state_with(100, 9999.0, 0);
        assert_eq!(StreamEndTermination.should_terminate(&s), Termination::Continue);
    }

    #[test]
    fn utterance_cap_fires_at_cap() {
        let t = UtteranceCapTermination { cap: 3 };
        assert_eq!(t.should_terminate(&state_with(2, 0.0, 1)), Termination::Continue);
        assert_eq!(
            t.should_terminate(&state_with(3, 0.0, 1)),
            Termination::Done("utterance_cap")
        );
    }

    #[test]
    fn audio_secs_fires_at_threshold() {
        let t = AudioSecsTermination { max_secs: 5.0 };
        assert_eq!(t.should_terminate(&state_with(0, 4.9, 1)), Termination::Continue);
        assert_eq!(
            t.should_terminate(&state_with(0, 5.0, 1)),
            Termination::Done("audio_secs")
        );
    }

    #[test]
    fn budget_fires_at_zero() {
        assert_eq!(
            BudgetTermination.should_terminate(&state_with(0, 0.0, 1)),
            Termination::Continue
        );
        assert_eq!(
            BudgetTermination.should_terminate(&state_with(0, 0.0, 0)),
            Termination::Done("budget_exhausted")
        );
    }

    #[test]
    fn composite_first_to_fire_wins() {
        let t = CompositeTermination::new(vec![
            Box::new(UtteranceCapTermination { cap: 10 }),
            Box::new(AudioSecsTermination { max_secs: 1.0 }),
        ]);
        assert_eq!(
            t.should_terminate(&state_with(2, 2.0, 1)),
            Termination::Done("audio_secs")
        );
        assert_eq!(t.should_terminate(&state_with(2, 0.5, 1)), Termination::Continue);
    }
}
