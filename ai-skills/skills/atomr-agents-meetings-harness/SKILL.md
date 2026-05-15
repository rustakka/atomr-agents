---
name: atomr-agents-meetings-harness
description: Use when turning a diarized STT transcript into structured meeting artifacts in atomr-agents — building a `MeetingsHarness`, accumulating a `MeetingAnalysis` (attendees, notes, actions-with-owners, tiered summaries), picking batch vs live mode, persisting via the configured `Checkpointer` under the same `conversation_id` as the source transcript, or serving the meetings review UI (`meetings-harness-web`). Triggers on `MeetingsHarness`, `MeetingAnalysis`, "extract action items", "meeting summary", "attendees from transcript", "live meeting analysis", `atomr_agents.meetings_harness`.
---

# Meetings harness

`atomr-agents-meetings-harness` turns a diarized `SttConversation` into
a structured `MeetingAnalysis`: an attendee roster, a linear
append-only ledger of notes and actions with owners, and a tiered
summary stack that regenerates dynamically as the meeting unfolds.

`atomr-agents-meetings-harness-web` is the optional axum + React review
UI on top of it (port 7100, side-by-side with the STT UI on 7000).

## When to use it

- You have a finished diarized transcript (an `SttConversation` from
  `stt-harness`) and want **attendees, notes, action items with
  owners, and a TL;DR**.
- You have a **live** STT run and want incremental analysis that grows
  monotonically (notes/actions append; tail-segment summary revises;
  earlier segments freeze) so a UI can stream it.
- You want both records (transcript + analysis) **joined under the same
  `conversation_id`** in whichever `Checkpointer` backend is configured.
- **Don't** use it for raw transcription — that's `stt-harness`.
- **Don't** use it for general-purpose summarization of arbitrary text
  — it expects the diarized turn structure.

## Build a `MeetingsHarness`

```rust
use std::sync::Arc;
use atomr_agents_meetings_harness::{
    BatchExtractionLoop, InMemoryMeetingsStore, IterationCapTermination,
    MeetingsHarness, MeetingsHarnessSpec, RuleBasedExtractor, RunMode,
};
use atomr_agents_stt_harness::{ConversationStore, InMemoryConversationStore};

let transcripts: Arc<dyn ConversationStore> =
    Arc::new(InMemoryConversationStore::new());
let analyses = Arc::new(InMemoryMeetingsStore::new());
let extractor = Arc::new(RuleBasedExtractor::new());

let spec = MeetingsHarnessSpec::new("meetings", "claude-opus-4-7")
    .with_mode(RunMode::Batch)
    .with_max_iterations(32);

let harness = MeetingsHarness::new(
    spec,
    transcripts.clone(),
    analyses.clone(),
    extractor,
    BatchExtractionLoop,
    IterationCapTermination::new(32),
);

let analysis = harness.run("call-7").await?;  // loads transcript by id
```

The **model id is required** on the spec — the caller picks it. The
default `RuleBasedExtractor` is deterministic and LLM-free (useful for
tests + the rest of the system end-to-end); production deployments swap
in their own `MeetingExtractor` impl that drives an `Agent`.

## Live mode

```rust
use atomr_agents_meetings_harness::{StreamingExtractionLoop, RunMode};

// Subscribe BEFORE the STT harness runs so no events are missed.
let stt_events = stt_harness.events();
let spec = MeetingsHarnessSpec::new("meetings", model_id)
    .with_mode(RunMode::Live { segment_turn_count: 8 });
let loop_strategy =
    StreamingExtractionLoop::new(stt_events, transcripts.clone(), "call-7".to_string());
let harness = MeetingsHarness::new(spec, transcripts, analyses, extractor, loop_strategy, term);
```

In live mode the loop:
- pulls new turns from the configured transcript store as the STT
  broadcast announces them,
- runs the extractor over the new window only,
- appends notes/actions to the tail (never reorders),
- revises the in-flight tail `SegmentSummary` until it exceeds
  `segment_turn_count`, then finalizes and opens a new one,
- recomposes `summary_levels.running` only when a segment finalizes,
- regenerates `summary_levels.tldr` only on `finalize`.

`harness.cancel()` (or `POST /api/meetings/:id/stop`) signals
cooperative cancellation.

## Data model

`MeetingAnalysis` holds `attendees`, `notes`, `actions`, and a
`SummaryLevels { segments, running, tldr }`. See
[`docs/meetings-harness.md`](../../../docs/meetings-harness.md) for the
full schema.

Foreign keys: `Action.owner_attendee_id` references `Attendee.id`. The
tool layer validates the reference on every insert/update.

## Tools

| Tool                   | Purpose                                       |
|------------------------|-----------------------------------------------|
| `list_turns`           | Page transcript turns.                        |
| `get_turn`             | Full detail for one turn.                     |
| `upsert_attendee`      | Idempotent attendee add/merge.                |
| `append_note`          | Append a note (linear, append-only).          |
| `append_action`        | Append an action (owner_id must resolve).     |
| `update_action`        | Patch a single action in place.               |
| `revise_tail_segment`  | Rewrite the in-flight tail segment summary.   |
| `finalize_segment`     | Freeze the tail; next revise opens a new one. |
| `regenerate_running`   | Recompute `summary_levels.running`.           |
| `set_title`            | Set/replace the meeting title.                |
| `finalize`             | Set the TL;DR and mark `state = Final`.       |

There is **no `delete_note` / `delete_action`** by design — append-only
is enforced at the tool surface.

## Persistence

`MeetingsStore` trait + `InMemoryMeetingsStore` (default) +
`CheckpointerMeetingsStore` (feature `state`). Analyses are filed under
`workflow_id = "meetings-harness"`, `run_id = analysis.id` — i.e. the
same `conversation_id` as the source `SttConversation`, so both records
join naturally.

## CLI

```bash
# Analyze an existing transcript (requires --features meetings).
atomr-agents meetings analyze \
    --conversation-id call-7 \
    --model claude-opus-4-7 \
    --mode batch

# Serve the review UI (requires --features meetings-web).
atomr-agents meetings serve --bind 127.0.0.1:7100
```

## See also

- [`atomr-agents-stt-harness`](../atomr-agents-stt-harness/SKILL.md) —
  produces the transcripts this harness consumes.
- [`docs/meetings-harness.md`](../../../docs/meetings-harness.md) —
  full architecture / data shapes / web API reference.
