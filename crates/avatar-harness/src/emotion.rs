//! Emotion actor — folds per-turn deltas into the running mood state.
//!
//! The state is shared between the cognition pipeline (which writes
//! emotion deltas after each reply) and the sync-manager (which reads
//! the latest mood to attach to outgoing frames).

use std::sync::Arc;

use parking_lot::RwLock;

use atomr_agents_avatar_core::{EmotionDelta, EmotionVector};

/// Holds the current [`EmotionVector`] under an [`RwLock`].
/// Cheap to clone — internally an `Arc<RwLock<_>>`.
#[derive(Clone, Default)]
pub struct EmotionState {
    inner: Arc<RwLock<EmotionVector>>,
    decay: f32,
}

impl EmotionState {
    /// `decay` is the per-update inertia (see
    /// [`EmotionVector::apply`]). Reasonable defaults: `0.4`–`0.7`.
    pub fn new(initial: EmotionVector, decay: f32) -> Self {
        Self {
            inner: Arc::new(RwLock::new(initial)),
            decay: decay.clamp(0.0, 1.0),
        }
    }

    /// Apply a delta, mutating the running state.
    pub fn apply(&self, delta: EmotionDelta) {
        self.inner.write().apply(delta, self.decay);
    }

    /// Snapshot the current state.
    pub fn snapshot(&self) -> EmotionVector {
        *self.inner.read()
    }

    /// Hard-reset to neutral.
    pub fn reset(&self) {
        *self.inner.write() = EmotionVector::neutral();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applying_deltas_moves_state() {
        let state = EmotionState::new(EmotionVector::neutral(), 0.5);
        state.apply(EmotionDelta {
            valence: 1.0,
            arousal: 0.0,
            anger: 0.0,
            surprise: 0.0,
            tension: 0.0,
        });
        let snap = state.snapshot();
        assert!(snap.valence > 0.0 && snap.valence <= 1.0);
    }

    #[test]
    fn reset_returns_to_neutral() {
        let state = EmotionState::new(
            EmotionVector {
                valence: 0.9,
                arousal: 0.5,
                anger: 0.0,
                surprise: 0.0,
                tension: 0.0,
            },
            0.5,
        );
        state.reset();
        assert_eq!(state.snapshot(), EmotionVector::neutral());
    }
}
