# Skills (M4)

M1 already parses `SKILL.md` files into native `Skill`/`SkillSet`
handles. M4 adds:

* a **keyword selector** that picks which skills are active for a
  given user message,
* a **validator** that surfaces parse problems without aborting on the
  first failure,
* CLI subcommands `atomr-host skill new / ls / validate`.

## Authoring a skill

```bash
atomr-host skill new default summarize \
    --keywords "summarize,tldr,condense" --priority 8
# wrote /.../agents/default/skills/summarize/SKILL.md
```

The scaffolded file:

```markdown
---
name: Summarize
priority: 8
keywords:
  - summarize
  - tldr
  - condense
tool_overlay: []
memory_namespace: []
---

# Summarize

Replace this paragraph with the skill body. It becomes the skill's
`instruction_fragment` and is concatenated into the system prompt
when the skill is selected by the keyword strategy.
```

Edit the body in your editor of choice — that text is the
`instruction_fragment` the model sees when the skill activates.

## Listing skills

```bash
atomr-host skill ls default
# summarize  priority=8  keywords=[summarize, tldr, condense]  name='Summarize'

atomr-host skill ls default --format json
# [{"id":"summarize","name":"Summarize","priority":8,"keywords":[...]}]
```

## Validating skills

`validate` walks every `skills/<id>/SKILL.md` and reports errors and
warnings independently — one bad skill doesn't mask the rest. Errors
imply the skill won't load; warnings are advisories.

```bash
atomr-host skill validate default
# [OK] summarize  (.../skills/summarize/SKILL.md)
```

Exit code is `0` when every skill passes, `1` otherwise — usable as a
CI gate (`atomr-host skill validate default || exit 1`).

JSON output is structured for tooling:

```bash
atomr-host skill validate default --format json
# [{"skill_id":"summarize","path":"...","ok":true,"errors":[],"warnings":[]}]
```

## Programmatic selection

```python
from atomr_agents.agent_host import AgentLoader, HostConfig, select_skills_for

defn = AgentLoader(HostConfig.load_default()).parse("default")
active = select_skills_for(defn.skills, "please give me a TLDR")
for s in active:
    print(s.id, s.priority, s.keywords)
```

Semantics:

- Case-insensitive substring match against each skill's `keywords`.
- A skill with an empty `keywords` list is never selected.
- Results are sorted by `priority` descending, then `id` ascending.

## How M4 integrates with the chat preview (M2)

`render_chat_preview` now lists active skill ids alongside the rule /
memory / skill counts:

```
[default] A helpful general-purpose assistant.
user: please give me a tldr
rules: 3 | memory facts: 1 | skills: 1 (active: summarize)
```

When you swap the preview responder for a real `InferenceClient`
(M9), the active skill list is what you'd pass to a
`keyword_skill_strategy` or splice into the prompt directly. The
selector is built so that change is local — it doesn't ripple into
the chat callable contract.

## Native KeywordSkillStrategy

For the eventual real-LLM responder, `build_keyword_skill_strategy`
returns the native `SkillStrategy` handle directly:

```python
from atomr_agents.agent_host import build_keyword_skill_strategy

loaded = AgentLoader(HostConfig.load_default()).load("default")
strategy = build_keyword_skill_strategy(loaded)
# pass `strategy` to an Agent that consumes a SkillStrategy
```

This mirrors `_native.skill.keyword_skill_strategy` semantics so the
Python selector and the native runtime agree on which skills
activate.
