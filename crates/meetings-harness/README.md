# atomr-agents-meetings-harness

Downstream harness that consumes a diarized `SttConversation` produced by
`stt-harness` and emits a structured [`MeetingAnalysis`]: attendees,
linear notes, action items with owners, and a tiered summary ledger.

Two run modes:

- **Batch** — analyze a complete transcript in one pass.
- **Live** — subscribe to a running STT harness's event stream and
  incrementally update the analysis as new turns commit. The notes and
  actions ledgers are **append-only** and **monotonic**, so the UI can
  render new entries without reflow. Summaries are **tiered**: per-segment
  summaries are revised only while their segment is the in-flight tail,
  then frozen; a running rollup recomposes on segment finalization; a
  TL;DR is regenerated on `finalize`.

The analysis is persisted under the **same `conversation_id` as the
input transcript**, through whichever
[`atomr_agents_state::Checkpointer`] backend is configured.

See [`docs/meetings-harness.md`](../../docs/meetings-harness.md) for the
full design.
