# Eval harness wiring (M12)

`atomr-host eval` runs YAML/JSON eval suites against an assembled
host agent. Suites live at `<root>/evals/<suite_id>.yaml` and are
discoverable / scaffoldable from the CLI.

## Suite format

```yaml
# <root>/evals/smoke.yaml
id: smoke
scorer: contains
description: Smoke check that the agent surfaces its identity.
cases:
  - id: identity
    input: hello
    expected:
      contains: ["default"]
  - id: rules_present
    input: what should you do
    expected:
      contains: ["rules:", "memory facts:"]
```

`scorer` is one of:

| Scorer       | `expected` shape          | Passes when                       |
|--------------|---------------------------|-----------------------------------|
| `contains`   | `{"contains": ["s1","s2"]}` | every substring appears in output |
| `excludes`   | `{"excludes": ["s1"]}`      | every substring is absent         |
| `regex`      | `{"regex": "pattern"}`      | `re.search(pattern, output)` hits |

Plugging in a custom scorer is one line: append it to `SCORERS` in
`atomr_agents.agent_host.evals` before calling `run_suite`.

## CLI

```bash
atomr-host eval new smoke              # scaffolds <root>/evals/smoke.yaml
atomr-host eval ls
atomr-host eval run default smoke      # runs the suite against the `default` agent
# [PASS] identity        score=1.00
# [FAIL] rules_present   score=0.50 — missing 1/2 substring(s): ['memory facts:']
# suite=smoke agent=default  passed=1/2  pass_rate=50.00%
```

Exit code is `0` on full pass, `1` otherwise — CI-ready.

## Programmatic API

```python
from atomr_agents.agent_host import HostConfig, load_suite, run_suite_sync

cfg = HostConfig.load_default()
suite = load_suite(cfg, "smoke")
run = run_suite_sync(cfg, "default", suite)
print(run.pass_rate, [r.case_id for r in run.results if not r.passed])
```

By default the runner uses `render_chat_preview` as the responder —
that's the deterministic M2 preview text, pure-Python, no LLM
required. Swap in a real responder when an inference client is
wired:

```python
def my_responder(loaded, user_msg: str) -> str:
    return call_openai(loaded, user_msg)

run = run_suite_sync(cfg, "default", suite, responder=my_responder)
```

## Why pure-Python by default?

Eval suites should be testable in CI without API keys. The M2
deterministic responder always emits the persona identity, the
counts of rules / memory / skills, and the user message — that's
enough surface to write meaningful smoke checks like:

- "the loaded persona is the one I expected"
- "the active skill list contains `summarize` for input `please tldr`"
- "no leaked secret patterns end up in the output" (`excludes`)

For richer evals over a real LLM, pass `responder=` to `run_suite`.

## Integration with the existing eval crate

`crates/eval` ships native scorers (`RubricScorer`, `LlmJudgeScorer`,
regression gates). M12's surface deliberately overlaps in
*intent* without rewriting those primitives — the on-disk YAML form,
discoverable suites, and CLI runner are host-level conveniences.
Once the runner can call the native scorers through PyO3, the
`SCORERS` registry will grow `rubric`, `llm_judge`, etc. The shape of
`run_suite` doesn't change.
