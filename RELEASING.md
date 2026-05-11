# Releasing atomr-agents

Quick reference for cutting a release. The deep dives live in:

* [`docs/release-pipeline.md`](docs/release-pipeline.md) — workflow
  internals (jobs, matrix, build commands).
* [`docs/release-process.md`](docs/release-process.md) — operator-
  facing reference (trampoline architecture, troubleshooting,
  Conventional-Commit rules).

```
Conventional-Commit on main
        │
        ▼
.github/workflows/version-bump.yml
        │  decides patch / minor / major / skip
        │  bumps Cargo.toml + Cargo.lock + pyproject.toml
        │  commits `chore(release): vX.Y.Z`
        │  tags `vX.Y.Z` and pushes
        │  ⤷ gh workflow run release.yml --ref vX.Y.Z   ← trampoline
        ▼
.github/workflows/release.yml
        │  verify (build + test gate)
        │  build-binaries (5 targets)
        │  build-wheels (6 platforms × CPython 3.10–3.13)
        │  build-sdist
        │  package-skills (ai-skills.tar.gz)
        │  github-release
        │  publish-crates              ← dep-order, allowlist-gated
        │  publish-pypi                ← OIDC trusted publishing
        ▼
   crates.io + PyPI + GitHub Release
```

## Conventional-Commit rules

| Subject prefix | Bump |
|---|---|
| `feat: …` | minor |
| `fix: …` / `perf: …` / `revert: …` | patch |
| `BREAKING CHANGE` body or `!:` after type | major |
| `chore:` / `docs:` / `ci:` / `test:` / `refactor:` / `style:` / `build:` only | skip |

A footer `Release-As: x.y.z` overrides auto-decision and pins the
exact version.

## Crate publish order (48 crates)

The `publish-crates` job walks every publishable workspace member in
strict dependency order, with a 70s pace between successful publishes
(crates.io rate limits new crates at ~1/min) and an exponential-
backoff retry on `429 Too Many Requests`.

```
Layer  Crate(s)
─────  ──────────────────────────────────────────────────────────────
  1    atomr-agents-core
  2    atomr-agents-callable
  3    atomr-agents-strategy
  4    atomr-agents-context
  5    atomr-agents-state
  6    atomr-agents-observability
  7    atomr-agents-tool
  8    atomr-agents-skill
  9    atomr-agents-memory
 10    atomr-agents-embed
 11    atomr-agents-retriever
 12    atomr-agents-ingest
 13    atomr-agents-persona
 14    atomr-agents-instruction
 15    atomr-agents-cache
 16    atomr-agents-parser
 17    atomr-agents-agent
 18    atomr-agents-workflow
 19    atomr-agents-harness
 20    atomr-agents-org
 21    atomr-agents-registry
 22    atomr-agents-eval
 23    atomr-agents-testkit
       ── speech-to-text capability ─────────────────────────────────
 24    atomr-agents-stt-core
 25    atomr-agents-stt-remote-core
 26    atomr-agents-stt-audio
 27    atomr-agents-stt-runtime-openai
 28    atomr-agents-stt-runtime-deepgram
 29    atomr-agents-stt-runtime-assemblyai
 30    atomr-agents-stt-runtime-whisper
 31    atomr-agents-stt-diarize-sherpa
 32    atomr-agents-stt-voice
 33    atomr-agents-stt-tool
       ── text-to-speech capability ─────────────────────────────────
 34    atomr-agents-tts-core
 35    atomr-agents-tts-audio
 36    atomr-agents-tts-runtime-openai
 37    atomr-agents-tts-runtime-elevenlabs
 38    atomr-agents-tts-runtime-openai-realtime
 39    atomr-agents-tts-runtime-gemini-live
 40    atomr-agents-tts-runtime-piper
 41    atomr-agents-tts-runtime-kokoro
 42    atomr-agents-tts-runtime-moss
 43    atomr-agents-tts-runtime-xtts
 44    atomr-agents-tts-voice
 45    atomr-agents-tts-tool
       ──────────────────────────────────────────────────────────────
 46    atomr-agents-py-bindings
 47    atomr-agents-cli
 48    atomr-agents (umbrella)
```

`xtask` is `publish = false` and never goes to crates.io.

The repo variable `ATOMR_AGENTS_PUBLISH_ALLOWLIST` (space-separated
crate names) overrides the default order. Set it to the empty string
to skip publish entirely; set it to a subset to ship only those
crates (useful for republish recovery).

## Manual operations

```bash
# Dry-run a release: builds artifacts, dry-run publishes, uploads to TestPyPI.
gh workflow run release.yml -f dry_run=true

# Skip Python: cargo-only release.
gh workflow run release.yml -f dry_run=true -f skip_python=true

# Skip Rust: wheels-only release.
gh workflow run release.yml -f dry_run=true -f skip_crates=true

# Force a bump kind from version-bump.yml (when commits would otherwise skip).
gh workflow run version-bump.yml -f force=patch     # or minor / major

# Pin to an exact version.
gh workflow run version-bump.yml -f release_as=0.7.5

# Cut a release without the trampoline (for tags that predate it).
git tag v0.9.3 <sha>
git push origin v0.9.3
```

## Pre-flight checklist

Before tagging a release, run the pre-flight locally:

```bash
# 1. Workspace builds clean.
cargo check --workspace --all-features

# 2. All tests pass.
cargo test --workspace

# 3. Each crate dry-runs `cargo publish`.
cargo publish -p atomr-agents-core --dry-run
# … repeat through the publish order …

# 4. Documentation builds.
cargo doc --workspace --no-deps

# 5. Backend feature flags compile.
cargo check -p atomr-agents-state --features sqlite,postgres
cargo check -p atomr-agents-memory --features pgvector,qdrant,chroma
cargo check -p atomr-agents-cache --features sqlite,redis

# 6. Umbrella in three configurations.
cargo build -p atomr-agents
cargo build -p atomr-agents --no-default-features
cargo build -p atomr-agents --all-features
```

## Per-crate metadata requirements

Every publishable crate needs:

- `description`
- `keywords` (≤ 5)
- `categories` (one or two from <https://crates.io/category_slugs>)
- `repository`
- `homepage`
- `license` (`Apache-2.0`)
- `readme = "../../README.md"` (or a per-crate README if the crate's
  surface is meaningfully distinct)

The workspace `[workspace.package]` already supplies `version`,
`edition`, `rust-version`, `license`, `repository`, `homepage`, and
`authors`. Per-crate `Cargo.toml`s add `description`, `keywords`,
`categories`, and `readme`.

## Sibling workspace deps

`atomr-agents` consumes the sibling `atomr`, `atomr-infer`, and
`atomr-accel` workspaces as **public crates.io dependencies only** —
never path links. Bumping a sibling version requires that version to
already be on crates.io; the release pipeline does not check out
sibling repos.

## Python (PyPI)

The `atomr-agents` Python wheel is built from
`crates/py-bindings/Cargo.toml` via maturin and published to
<https://pypi.org/p/atomr-agents>. Wheels cover CPython 3.10–3.13
across:

* `manylinux_2_17` x86_64 + aarch64 (glibc Linux)
* `musllinux_1_2` x86_64 + aarch64 (Alpine-friendly)
* `macosx_*_universal2` (fat: x86_64 + arm64)
* `win_amd64`

Authentication uses [PyPI Trusted Publishing](https://docs.pypi.org/trusted-publishers/),
not an API token. See `docs/release-process.md` for one-time setup.

## Marketplace (`ai-skills`)

The plugin marketplace publish runs `gh release upload <tag>
ai-skills.tar.gz` so consumers can `/plugin install
atomr-agents-ai-skills@atomr-agents` from the released artifact.

## First release

The first published version is **0.1.0**. Subsequent semver applies
normally. The `version-bump.yml` job refuses to auto-bump from no
prior tag — the first release must come from
`workflow_dispatch -f release_as=0.1.0` (or a `Release-As: 0.1.0`
footer on the head commit).

## Yanking

If a release has a critical bug:

```bash
cargo yank --vers x.y.z atomr-agents-<crate>
```

Yank from leaves up (umbrella → cli → py-bindings → tts-* → stt-* →
testkit → eval → registry → harness → workflow → agent → … → core),
so dependent versions don't briefly fail to resolve.
