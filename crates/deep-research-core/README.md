# atomr-agents-deep-research-core

Pure data types for the deep-research harness. Holds the uniform input
(`ResearchRequest`) and output (`ResearchResult`) shapes that every
strategy under `atomr-agents-deep-research-harness` produces, plus
supporting types: citations, plans, sub-questions, transcript node
steps, coverage signals, telemetry, and artifacts.

This crate is intentionally runtime-free so callers (CLIs, UIs, Python
bindings) can ship the contract without pulling in the harness runtime.
