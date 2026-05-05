//! Channelled state, reducers, and per-super-step checkpointing.
//!
//! Implements LangGraph's StateGraph state model in atomr-agents
//! idioms. A `StateSchema` declares a channel per state key; each
//! channel carries a reducer that merges the current value with
//! incoming writes. `RunState` holds the values and exposes
//! `merge_writes` for batch application after each super-step. A
//! pluggable `Checkpointer` persists snapshots so workflows can
//! resume, replay, and fork.

mod backends;
mod checkpointer;
mod reducer;
mod schema;
mod state;

pub use checkpointer::{CheckpointKey, CheckpointMeta, Checkpointer, InMemoryCheckpointer, Snapshot};
pub use reducer::{
    reducer_box, AppendList, AppendMessages, DynReducer, LastWriteWins, MaxByTimestamp, MergeMap, Reducer,
};
pub use schema::{Channel, StateSchema, StateSchemaBuilder};
pub use state::RunState;

#[cfg(feature = "postgres")]
pub use backends::postgres::PostgresCheckpointer;
#[cfg(feature = "sqlite")]
pub use backends::sqlite::SqliteCheckpointer;
