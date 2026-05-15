# atomr-agents-coding-cli-vendor-gemini

Google Gemini CLI adapter for the coding-cli harness.

## Headless invocation

```text
gemini -p <prompt>
  --output-format stream-json
  --non-interactive
  [--model <id>]
  [--yolo]    # if PolicySnapshot.auto_approve_unrestricted
```

## Concept projection

Gemini lacks a native skill / agent registry, so persona, skills, and
project memory are concatenated into a single
`<workdir>/.gemini/system_instructions.md` file. MCP servers land in
`<workdir>/.gemini/settings.json` under the `mcpServers` key.
