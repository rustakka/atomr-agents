# atomr-agents-coding-cli-vendor-antigravity

Google Antigravity CLI (`agy`) adapter for the coding-cli harness.
Successor to the Gemini CLI adapter — the legacy `gemini` CLI is
deprecated and stops serving requests on 2026-06-18.

Unlike the old Gemini CLI, Antigravity serves **non-Gemini models**
(Claude, etc.) through Google's `cloudcode-pa.googleapis.com` backend,
selectable via the model flag, so a single vendor can drive multiple
model families.

Install the CLI with:

```bash
curl -fsSL https://antigravity.google/cli/install.sh | bash
# installs to ~/.local/bin/agy
```

## Headless invocation

```text
agy -p <prompt>
  --output-format stream-json
  --non-interactive
  [--model <id>]    # any model id, including non-Gemini (e.g. Claude)
  [--yolo]          # if PolicySnapshot.auto_approve_unrestricted
```

The binary name, flags, and on-disk config layout are all configurable
via `AntigravityConfig` (the defaults above mirror the Gemini CLI
surface). Operators can correct any field — e.g. once `agy --help`
documents its real headless flags — without a code change:
`AntigravityVendor::with_config(AntigravityConfig { .. })`.

## Concept projection

Antigravity lacks a native skill / agent registry, so persona, skills,
and project memory are concatenated into a single
`<workdir>/.antigravity/system_instructions.md` file. MCP servers land
in `<workdir>/.antigravity/settings.json` under the `mcpServers` key.
The config directory and filenames are overridable via
`AntigravityConfig`.
