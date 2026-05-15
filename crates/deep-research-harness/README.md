# atomr-agents-deep-research-harness

A pluggable deep-research harness with three v1 strategies modelled after
NVIDIA AI-Q (clarify → plan → search → write → verify), Anthropic's
multi-agent research system (lead + parallel subagents), and LangGraph's
`open_deep_research` (supervisor with `think_tool`).

All three strategies emit the same `ResearchResult` shape (from
[`atomr_agents_deep_research_core`]) so callers can swap topologies
without changing the surrounding code. Role implementations
(`Clarifier`, `Planner`, `Researcher`, `Writer`, `Critic`,
`CitationVerifier`) are pluggable; deterministic LLM-free defaults
(`TemplateClarifier`, `HeuristicPlanner`, `MockResearcher`,
`ConcatWriter`, `RegexCritic`, `DeterministicCitationVerifier`) ship
in the crate so tests and the web UI exercise end-to-end without a
model provider.

See `docs/deep-research-harness.md` for design and usage.
