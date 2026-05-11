# Release pipeline

> **See also:** [release-process.md](release-process.md) — the
> operator-facing reference (how to ship, conventional-commit rules,
> trampoline architecture, troubleshooting). This document focuses on
> workflow internals: jobs, matrix entries, build commands.

`/.github/workflows/release.yml` ships atomr-agents to three places on
every `v*` tag:

1. **GitHub Releases** — pre-built `atomr-agents` binaries, all built
   Python wheels, and the `ai-skills.tar.gz` marketplace artifact.
2. **crates.io** — every publishable Rust crate (48 in total), in
   dependency order.
3. **PyPI** — platform-specific wheels (Linux glibc x86_64/aarch64,
   Linux musl x86_64/aarch64, macOS universal2, Windows x86_64) and
   an sdist.

## Triggering

There are three paths into this pipeline; they all converge on the
same publish jobs.

* **Direct tag push** (`git push origin vX.Y.Z`) — fires
  `on: push: tags`. Use this when a human is cutting a release
  outside of the auto-bump flow.
* **Auto-bump trampoline** — `version-bump.yml` runs on every push
  to `main` and decides a SemVer bump from Conventional-Commit
  subjects (`feat:` → minor, `fix:`/`perf:`/`revert:` → patch,
  `!:`/`BREAKING CHANGE` → major; everything else — including
  `build:`, `chore:`, `docs:`, `ci:`, `test:`, `refactor:`,
  `style:` — is `skip`). When it decides to bump, it commits the
  version change, tags it, pushes, **and then explicitly dispatches
  `release.yml`** via `gh workflow run release.yml --ref vX.Y.Z
  -f dry_run=false`. The explicit dispatch is required because tag
  events authored by the default `GITHUB_TOKEN` do not fire downstream
  workflows.
* **Manual `workflow_dispatch`** — choose `dry_run=true` for a
  rehearsal that publishes to TestPyPI and runs `cargo publish --dry-run`.
  Toggle `skip_python` / `skip_crates` to ship to only one registry.
  A manual dispatch with `dry_run=false` against a `v*` tag ref also
  performs a real publish (this is the same path the trampoline takes).

### What gets published when

| Trigger | verify | binaries | wheels | GitHub Release | crates.io | PyPI |
|---|---|---|---|---|---|---|
| `push` on `v*` tag | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `workflow_dispatch` ref=`v*` `dry_run=false` | ✓ | ✓ | ✓ | ✓ | ✓ (unless `skip_crates`) | ✓ (unless `skip_python`) |
| `workflow_dispatch` `dry_run=true` | ✓ | ✓ | ✓ | — | dry-run only | TestPyPI |

The publish jobs guard on `startsWith(github.ref, 'refs/tags/v')`, so
a `workflow_dispatch` against a branch ref will only run the verify
gate and (optionally) dry-run jobs — never a real publish.

## What gets built

### Binaries (`build-binaries`)

Single binary `atomr-agents` (from `crates/cli`). Cross-compiled for:

| OS | Target | Notes |
|---|---|---|
| Ubuntu | `x86_64-unknown-linux-gnu` | native cargo |
| Ubuntu (ARM runner) | `aarch64-unknown-linux-gnu` | native cargo on `ubuntu-22.04-arm` |
| macOS | `x86_64-apple-darwin` | native cargo |
| macOS | `aarch64-apple-darwin` | native cargo |
| Windows | `x86_64-pc-windows-msvc` | native cargo |

aarch64-Linux is built natively on a GitHub-hosted ARM runner rather
than via `cross` — this avoids the `ring` / `aws-lc-rs` / `openssl`
cross-compile blockers and dodges the cross-rs path-dep mount issue
documented in the atomr-infer history.

### Wheels (`build-wheels`)

Built via `PyO3/maturin-action`. The action runs each target inside the
appropriate `manylinux` / `musllinux` container; the action's
`--interpreter` flag builds a wheel per CPython ABI (3.10 – 3.13).
The maturin manifest path is `crates/py-bindings/Cargo.toml`.

| OS | Target | Wheel tag |
|---|---|---|
| Ubuntu | `x86_64-unknown-linux-gnu` | `manylinux_2_17_x86_64` |
| Ubuntu (ARM) | `aarch64-unknown-linux-gnu` | `manylinux_2_17_aarch64` |
| Ubuntu | `x86_64-unknown-linux-musl` | `musllinux_1_2_x86_64` |
| Ubuntu (ARM) | `aarch64-unknown-linux-musl` | `musllinux_1_2_aarch64` |
| macOS | `universal2-apple-darwin` | `macosx_*_universal2` (fat: x86_64 + arm64) |
| Windows | `x86_64-pc-windows-msvc` | `win_amd64` |

### sdist (`build-sdist`)

A single source distribution `atomr_agents-X.Y.Z.tar.gz`, used by PyPI
for platforms that have no pre-built wheel.

### ai-skills (`package-skills`)

A tarball of the `ai-skills/` plugin marketplace folder, attached to
the GitHub Release. Consumers install via `/plugin install
atomr-agents-ai-skills@atomr-agents`.

## Required secrets / config

| Secret / variable | Where | Used by |
|---|---|---|
| `CRATES_IO_TOKEN` | repo `Settings → Secrets → Actions` | `publish-crates` |
| PyPI Trusted Publisher | configured on PyPI itself, **not** as a GitHub secret | `publish-pypi` |
| `ATOMR_AGENTS_PUBLISH_ALLOWLIST` (optional) | repo `Settings → Variables` | `publish-crates` (overrides default order) |

### PyPI Trusted Publishing setup

Trusted publishing avoids long-lived API tokens. One-time setup:

1. Create the project on https://pypi.org/manage/projects/ (or run a
   manual upload first).
2. Go to *Manage → Publishing → Add a new publisher → GitHub*.
3. Fill in:
   * Owner: `rustakka`
   * Repository: `atomr-agents`
   * Workflow name: `release.yml`
   * Environment: `pypi`
4. Repeat for TestPyPI with environment `testpypi` if you want
   dry-run uploads.

The `publish-pypi` job already declares `permissions: id-token: write`
and `environment: pypi` so the OIDC handshake works once you've
registered the publisher.

If you'd rather use an API token, replace the
`pypa/gh-action-pypi-publish` action's `with:` block with:

```yaml
with:
  packages-dir: upload
  password: ${{ secrets.PYPI_API_TOKEN }}
  skip-existing: true
```

## Crates published

The `publish-crates` job walks every publishable crate in dependency
order. Adding a new crate? Slot it into the earliest layer whose
prerequisites have already been published, and pin its intra-workspace
deps with `{ workspace = true }` (NOT a hand-written `version = "..."`
literal) so the next bump doesn't leave a stale pin behind.

Current order (top to bottom — 48 crates):

1. `atomr-agents-core`
2. `atomr-agents-callable`
3. `atomr-agents-strategy`
4. `atomr-agents-context`
5. `atomr-agents-state`
6. `atomr-agents-observability`
7. `atomr-agents-tool`
8. `atomr-agents-skill`
9. `atomr-agents-memory`
10. `atomr-agents-embed`
11. `atomr-agents-retriever`
12. `atomr-agents-ingest`
13. `atomr-agents-persona`
14. `atomr-agents-instruction`
15. `atomr-agents-cache`
16. `atomr-agents-parser`
17. `atomr-agents-agent`
18. `atomr-agents-workflow`
19. `atomr-agents-harness`
20. `atomr-agents-org`
21. `atomr-agents-registry`
22. `atomr-agents-eval`
23. `atomr-agents-testkit`
24. `atomr-agents-stt-core`
25. `atomr-agents-stt-remote-core`
26. `atomr-agents-stt-audio`
27. `atomr-agents-stt-runtime-openai`
28. `atomr-agents-stt-runtime-deepgram`
29. `atomr-agents-stt-runtime-assemblyai`
30. `atomr-agents-stt-runtime-whisper`
31. `atomr-agents-stt-diarize-sherpa`
32. `atomr-agents-stt-voice`
33. `atomr-agents-stt-tool`
34. `atomr-agents-tts-core`
35. `atomr-agents-tts-audio`
36. `atomr-agents-tts-runtime-openai`
37. `atomr-agents-tts-runtime-elevenlabs`
38. `atomr-agents-tts-runtime-openai-realtime`
39. `atomr-agents-tts-runtime-gemini-live`
40. `atomr-agents-tts-runtime-piper`
41. `atomr-agents-tts-runtime-kokoro`
42. `atomr-agents-tts-runtime-moss`
43. `atomr-agents-tts-runtime-xtts`
44. `atomr-agents-tts-voice`
45. `atomr-agents-tts-tool`
46. `atomr-agents-py-bindings` (PyO3 cdylib; depends on every internal crate)
47. `atomr-agents-cli` (binary; depends on harness/eval/registry/state/core)
48. `atomr-agents` (umbrella; published last)

Workspace members deliberately excluded: `xtask` (carries
`publish = false`).

## Sibling workspace deps

`atomr-agents` depends on three sibling workspaces — `atomr`,
`atomr-infer`, `atomr-accel` — declared in `[workspace.dependencies]`
as **crates.io version pins only**. There are no `path = "../..."`
links to sibling checkouts, and the release workflow does not
`actions/checkout` any sibling repos.

This is deliberate. Sibling path-deps couple builds across repository
boundaries and force every CI run to pin to specific sibling SHAs.
Consuming siblings as published crates keeps the pipeline self-
contained: bumping a sibling version requires that version to already
be on crates.io, full stop.

For local development against an unreleased sibling, contributors use
`[patch.crates-io]` overrides in their personal
`~/.cargo/config.toml` rather than workspace-level path-deps.

## Cross-publishing constraints

* **crates.io publishes are sequential** — every dependent crate
  must wait for its dependencies to be visible. The `publish-crates`
  job orders them deliberately; if you add a new crate, slot it into
  the matching layer of that block.
* **`already uploaded` is treated as success** — re-tagging the same
  version (after fixing one mid-pipeline crate) is cheap; previously-
  uploaded crates skip in <1s.
* **Rate limiting** — crates.io rate-limits new crates at ~1/min for
  fresh accounts/orgs. The job paces successful publishes at 70s and
  retries `429 Too Many Requests` with a 620s sleep (10min + 20s
  slack), up to 12 attempts per crate. With 48 crates this caps the
  publish-crates job around 60 min in the steady state.
* **Wheel ABI tags** are baked in by maturin from the build container,
  so each matrix entry produces a different wheel tag. If you need
  more (e.g. PyPy), add another matrix line.
* **Universal2 macOS wheels** cover both Intel and Apple Silicon in a
  single artifact; that's why we don't run separate macOS x86_64 and
  aarch64 wheel builds.
* **musllinux** is Alpine-friendly; if you don't ship to Alpine, drop
  those matrix rows to halve Linux build time.

## Verifying a release locally

Dry-run a release by triggering the workflow manually:

```
gh workflow run release.yml -f dry_run=true
```

This runs the verify gate, builds every binary + wheel, runs
`cargo publish --dry-run` on a representative subset of crates, and
uploads to TestPyPI (`https://test.pypi.org/p/atomr-agents`) without
touching crates.io or production PyPI. The artifacts also land on the
workflow's *Artifacts* panel so you can download and smoke-test
before tagging.
