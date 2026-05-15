# Meetings harness

The meetings harness sits **downstream of the STT harness**. It consumes
a diarized [`SttConversation`](../crates/stt-harness/src/conversation.rs)
and produces a structured `MeetingAnalysis`: an attendee roster, a
**linear, append-only** ledger of notes and actions with owners, and a
**tiered, dynamically regenerated** summary stack.

The analysis is persisted under the **same `conversation_id` as the
input transcript**, through whichever `Checkpointer` backend is
configured — so the diarized transcript and its analysis join naturally
in the same store.

## The problem it solves

`stt-harness` produces raw turns with speaker tags. Nothing downstream
turned those into the structured artifacts a human consumer wants:
"who was on the call", "what was decided", "who owes what by when". A
meetings harness is that missing layer; `meetings-harness-web` is the
optional review UI on top of it.

## Shape

`MeetingsHarness` mirrors the workspace `BoxedX` pattern, exactly like
`SttHarness`:

- `MeetingsHarness<L, T>` — typed, monomorphic over loop and termination.
- `BoxedMeetingsHarness` — the type-erased twin.
- Both funnel into one shared loop body (`run_loop`).
- `MeetingsHarnessRef` is the public handle and implements
  [`Callable`](../crates/callable) — so a meetings harness drops into a
  `Pipeline`, a workflow step, or a team routing target.

```rust
use std::sync::Arc;
use atomr_agents_meetings_harness::{
    BatchExtractionLoop, InMemoryMeetingsStore, IterationCapTermination,
    MeetingsHarness, MeetingsHarnessSpec, RuleBasedExtractor, RunMode,
};
use atomr_agents_stt_harness::{ConversationStore, InMemoryConversationStore};

let transcripts: Arc<dyn ConversationStore> = Arc::new(InMemoryConversationStore::new());
let analyses = Arc::new(InMemoryMeetingsStore::new());
let extractor = Arc::new(RuleBasedExtractor::new());

let spec = MeetingsHarnessSpec::new("meetings", "claude-opus-4-7").with_mode(RunMode::Batch);
let harness = MeetingsHarness::new(
    spec,
    transcripts,
    analyses,
    extractor,
    BatchExtractionLoop,
    IterationCapTermination::new(32),
);

// Load the SttConversation by id from the configured transcript store
// and produce the analysis (persisted under the same id).
let analysis = harness.run("call-7").await?;
```

## Two run modes

### Batch (default)

The harness loads the full transcript once, runs the extractor over the
complete content, generates per-segment summaries → running rollup →
TL;DR, and persists a `Final` analysis.

Trigger via CLI:
```bash
atomr-agents meetings analyze \
    --conversation-id call-7 \
    --model claude-opus-4-7 \
    --mode batch
```

Trigger via web:
```
POST /api/meetings/call-7/run
{ "mode": "batch", "model_id": "claude-opus-4-7" }
```

### Live

The harness subscribes to a running `SttHarness`'s event broadcast and
updates the analysis as new turns commit. Three invariants the live
loop maintains:

1. **The notes/actions ledger is append-only and monotonic.** New
   entries land at the tail; existing entries may be patched in place
   (action status, owner, due date) but are never reordered or deleted.
   This lets the UI render new notes without reflow.
2. **The in-flight tail segment summary is revised; earlier segments
   are frozen.** Each `SegmentSummary` covers a contiguous block of
   turns. While it is the tail, its `text` is rewritten as more turns
   arrive. Once it crosses `segment_turn_count` turns, it is finalized
   and a fresh in-flight segment opens.
3. **Summaries are regenerated independently per level.** The running
   rollup is recomposed only when a segment finalizes. The TL;DR is
   regenerated only on `finalize`. New content invalidates only the
   affected tier — earlier work is reused.

```rust
use atomr_agents_meetings_harness::{StreamingExtractionLoop, RunMode};

let stt_events = stt_harness.events();
let spec = MeetingsHarnessSpec::new("meetings", "claude-opus-4-7")
    .with_mode(RunMode::Live { segment_turn_count: 8 });
let loop_strategy =
    StreamingExtractionLoop::new(stt_events, transcripts.clone(), "call-7".to_string());
let harness = MeetingsHarness::new(spec, transcripts, analyses, extractor, loop_strategy, term);
```

`POST /api/meetings/:id/stop` signals cooperative cancellation; the
loop terminates with reason `cancelled` at its next iteration.

## Data model

```rust
pub struct MeetingAnalysis {
    pub id: String,                       // == source SttConversation.id
    pub title: Option<String>,
    pub summary_levels: SummaryLevels,    // tiered, dynamically regenerated
    pub attendees: Vec<Attendee>,         // by display_name + speaker_tags
    pub notes: Vec<Note>,                 // append-only ledger
    pub actions: Vec<Action>,             // append-only ledger
    pub source_transcript_id: String,     // == id
    pub last_processed_turn_index: Option<u64>,   // live-mode watermark
    pub model_id: Option<String>,
    pub state: AnalysisState,             // Pending | Streaming | Final
    // ...
}

pub struct SummaryLevels {
    pub segments: Vec<SegmentSummary>,    // tail (last, finalized=false) is mutable
    pub running: Option<String>,          // rollup of finalized segments
    pub tldr: Option<String>,             // final TL;DR
}
```

`Action::owner_attendee_id` is a foreign key to `Attendee::id`. The
tool layer validates the reference on every insert/update so a stored
action always points at an attendee that exists.

## Agent tools

Every mutation an extractor (rule-based or LLM-driven) makes is exposed
as a [`Tool`](../crates/tool):

| Tool                  | Purpose                                                  |
|-----------------------|----------------------------------------------------------|
| `list_turns`          | Page transcript turns (supports `since_index`).          |
| `get_turn`            | Full detail for one turn.                                |
| `upsert_attendee`     | Idempotent attendee add/merge.                           |
| `append_note`         | Append to the linear notes ledger.                       |
| `append_action`       | Append to the linear actions ledger.                     |
| `update_action`       | Patch an existing action in place.                       |
| `revise_tail_segment` | Rewrite the in-flight tail segment.                      |
| `finalize_segment`    | Freeze the tail segment; the next revise opens a new one.|
| `regenerate_running`  | Recompute the running rollup.                            |
| `set_title`           | Set/replace the meeting title.                           |
| `finalize`            | Set the TL;DR and mark `state = Final`.                  |

There is **no** `delete_note` or `delete_action` tool: the append-only
invariant is enforced at the tool surface.

## Persistence

`MeetingsStore` is the trait the web layer and any caller use to list,
fetch, and mutate analyses:

- `InMemoryMeetingsStore` — process-local, the always-available default.
- `CheckpointerMeetingsStore` *(feature = `state`)* — routes through
  `crates/state`'s `Checkpointer`. The deployment's
  configured backend (in-memory, SQLite, Postgres) governs durability.

Analyses are filed under `workflow_id = "meetings-harness"`,
`run_id = analysis.id`, which is **the same `run_id`** the STT
transcript uses under `workflow_id = "stt-harness"`. Both records join
naturally in the same store.

```
let cp: Arc<dyn Checkpointer> = ...;
let transcripts = CheckpointerConversationStore::new(cp.clone());
let analyses    = CheckpointerMeetingsStore::new(cp);
```

## Web UI — `meetings-harness-web`

Mirrors `stt-harness-web`. axum + React/Vite, on port `7100` by default:

| Method   | Path                                              | Purpose                                  |
|----------|---------------------------------------------------|------------------------------------------|
| `GET`    | `/api/meetings`                                   | Summary rows.                            |
| `GET`    | `/api/meetings/:id`                               | Full analysis.                           |
| `DELETE` | `/api/meetings/:id`                               | Tombstone (works for any backend).       |
| `PUT`    | `/api/meetings/:id/attendees/:attendee_id`        | Rename / set role / set email.           |
| `PATCH`  | `/api/meetings/:id/actions/:action_id`            | Status / owner / due date.               |
| `POST`   | `/api/meetings/:id/run`                           | Trigger a fresh run.                     |
| `POST`   | `/api/meetings/:id/stop`                          | Cancel an in-flight run.                 |
| `GET`    | `/api/transcripts`                                | STT-side transcripts available for analysis. |
| `GET`    | `/ws`                                             | Live `MeetingsHarnessEvent` stream.      |

Run it:
```bash
# Build the SPA once, then run the axum server with rust-embed.
npm --prefix crates/meetings-harness-web/ui ci
npm --prefix crates/meetings-harness-web/ui run build
cargo run -p atomr-agents-cli \
    --features meetings-web \
    -- meetings serve --bind 127.0.0.1:7100
```

Dev mode (Vite proxies `/api` and `/ws`):
```bash
npm --prefix crates/meetings-harness-web/ui run dev    # :5174
cargo run -p atomr-agents-cli --features meetings-web -- meetings serve
```

## See also

- [`stt-harness`](stt-harness.md) — the upstream that produces the
  transcripts this harness consumes.
- [`state-and-checkpointing`](state-and-checkpointing.md) — how the
  configured persistence provider is chosen.
- [`crates/meetings-harness/src/analysis.rs`](../crates/meetings-harness/src/analysis.rs)
  — the authoritative data model.
