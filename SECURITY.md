# Security policy

## Reporting vulnerabilities

Report security issues privately to the maintainers via GitHub's
security-advisory flow:
<https://github.com/rustakka/atomr-agents/security/advisories/new>.

Please **do not file a public issue** for unpatched security
problems.

## Scope

atomr-agents itself does not handle authentication or perform
cryptographic operations. Most security-relevant code paths are in:

- **Provider runtimes** (atomr-infer): API keys, TLS, HTTP retries.
  Vulnerabilities affecting those should be reported to
  [atomr-infer](https://github.com/rustakka/atomr-infer).
- **Backend stubs** (`atomr-agents-state` SQLite/Postgres,
  `atomr-agents-memory` pgvector/qdrant/chroma,
  `atomr-agents-cache` redis): each is a stub in this repo. Real
  wiring lives in deployment patches; report there.
- **Python bindings** (`atomr-agents-py-bindings`): GIL containment
  and FFI safety inherit from atomr's pycore. Issues affecting
  cross-FFI memory safety should be reported here.

## Surface that is in-scope for this repo

- Tool-call parser (OpenAI / Anthropic delta JSON ingestion).
- `serde_json::Value` paths (input from models).
- Channel reducers (associativity, panic-on-bad-input).
- Workflow event log / journal serde.
- Eval scorer code paths that re-prompt models with raw output.
- HITL `Command` resume API (state-edit auth is the caller's
  responsibility — this repo doesn't authenticate operators).

## What we treat as a security issue

- Memory unsafety in any unsafe block.
- DoS via crafted tool args / parser inputs.
- Path traversal in document loaders.
- Information disclosure in error messages.
- Backend stubs that silently succeed instead of returning the
  documented "feature not enabled" error.

## What we do not treat as a security issue

- Prompt-injection-style behavior of the underlying LLM (out of
  scope; the framework forwards user input to a model that may
  follow injected instructions).
- Eval scorer producing false positives / negatives (model quality,
  not framework correctness).
- Side-channel attacks via timing on `EventBus` subscribers.
