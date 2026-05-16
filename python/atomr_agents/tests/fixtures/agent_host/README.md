# agent_host test fixtures

Fixture host root used by `test_host_loader.py`. Mirrors the on-disk
layout in `python/atomr_agents/agent_host/layout.py`.

```
fixtures/agent_host/
├── config.yaml
├── AGENTS.md
└── agents/
    └── alpha/
        ├── agent.yaml
        ├── SOUL.md
        ├── RULES.md
        ├── MEMORY.md
        ├── USER.md
        ├── skills/
        │   └── summarize/SKILL.md
        └── hooks/
            └── on_tool_call.yaml
```
