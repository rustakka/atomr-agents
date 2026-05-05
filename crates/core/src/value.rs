/// The framework's open-ended value type. Currently a re-export of
/// `serde_json::Value`; the alias gives us a single point of change if
/// we want a stricter representation later.
pub type Value = serde_json::Value;

/// Re-export under a friendlier name for convenience.
pub use serde_json as Json;
