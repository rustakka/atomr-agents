# Releasing atomr-agents

The release pipeline mirrors atomr / atomr-infer's: a Conventional-
Commit subject on `main` triggers an automated bump, tag, and
crates.io publish. Day-to-day, contributors only need to write the
right commit subject.

```
Conventional-Commit on main
        │
        ▼
.github/workflows/version-bump.yml
        │  decides patch / minor / major / skip
        │  bumps Cargo.toml + Cargo.lock
        │  commits `chore(release): vX.Y.Z`
        │  tags `vX.Y.Z`
        │  pushes
        ▼
.github/workflows/release.yml   (fires on tag push)
        │  cargo xtask verify
        │  github-release
        │  publish-crates              ← dep-order, allowlist-gated
        ▼
   crates.io + GitHub Release
```

## Conventional-Commit conventions

| Subject prefix | Bump |
|---|---|
| `feat: …` | minor |
| `fix: …` / `perf: …` / `revert: …` | patch |
| `BREAKING CHANGE` body or `!:` after type | major |
| `chore:` / `docs:` / `ci:` / `test:` / `refactor:` / `style:` / `build:` only | skip |

A footer `Release-As: x.y.z` overrides auto-decision and pins the
exact version.

## Crate publish order

The publishing job walks the dep graph topologically. The order is:

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
24. `atomr-agents-py-bindings`
25. `atomr-agents-cli`
26. `atomr-agents` (umbrella)

`xtask` is `publish = false` and never goes to crates.io.

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

## Python (PyPI)

The `atomr-agents` Python wheel is built from
`crates/py-bindings/Cargo.toml` via maturin. The PyPI release lives
behind the same tag — `release.yml` runs `maturin publish` after
the crates.io publish step.

```bash
pip install maturin
maturin build --release --manifest-path crates/py-bindings/Cargo.toml
# — uploads to PyPI in CI —
```

## Marketplace (`ai-skills`)

The plugin marketplace publish runs `gh release upload <tag>
ai-skills.tar.gz` so consumers can `/plugin install
atomr-agents-ai-skills@atomr-agents` from the released artifact.

## First release

The first published version is **0.1.0**. Subsequent semver applies
normally.

## Yanking

If a release has a critical bug:

```bash
cargo yank --vers x.y.z atomr-agents-<crate>
```

Yank from leaves up (CLI → umbrella → eval → registry → harness →
workflow → agent → … → core), so dependent versions don't briefly
fail to resolve.
