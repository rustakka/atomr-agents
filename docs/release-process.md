# Release process

This document is the operator-facing reference for cutting an
`atomr-agents` release. It covers the day-to-day workflow ("I want to
ship a fix"), the architecture that makes auto-publish work, and the
recovery playbook when something goes sideways.

For workflow-internal detail (job names, matrix entries, build
commands), see [release-pipeline.md](release-pipeline.md). This
document is more pedagogical and self-contained — readable by someone
who hasn't seen the workflows before.

## TL;DR

* You release by **landing a Conventional-Commit-typed commit on `main`**.
  `feat:`, `fix:`, `perf:`, `revert:`, `!:` (breaking), or a
  `Release-As: x.y.z` footer triggers a bump and a full publish to
  crates.io + PyPI + GitHub Releases. Everything else (`build:`,
  `chore:`, `docs:`, `ci:`, `test:`, `refactor:`, `style:`) is a
  no-op.
* You do **not** push tags by hand and do **not** dispatch the release
  workflow by hand. The bump-and-tag bot does both.
* End-to-end takes ~75 minutes from `git push` to "all 48 crates on
  crates.io and the wheel set on PyPI." The crates.io publish loop
  dominates because of the new-crate rate limit (70s pacing × 48
  crates ≈ 56 min in steady state).
* If you need to ship something that isn't a real fix/feat, append a
  `Release-As: x.y.z` footer to any commit body to force an exact-
  version bump.

## The release loop, end to end

```
                 ┌────────────────────────────────────────────────────┐
   git push      │                                                    │
   (commit on    │  version-bump.yml             release.yml          │
   main)         │  ─────────────────            ──────────────       │
       │        │                                                    │
       ▼        │   1. read commits since                            │
  ┌────────┐   │      last tag                                       │
  │ main   │──▶│   2. decide bump kind                               │
  │ branch │   │   3. cargo set-version --workspace                  │
  └────────┘   │   4. bump pyproject.toml                            │
                │   5. commit chore(release): vX.Y.Z                  │
                │   6. tag vX.Y.Z + push                              │
                │   7. gh workflow run release.yml ──┐                │
                │      --ref vX.Y.Z                  │                │
                │      -f dry_run=false              ▼                │
                │                                                    │
                │                              1. verify gate         │
                │                              2. build binaries (×5) │
                │                              3. build wheels (×6 ×  │
                │                                 4 ABIs) + sdist     │
                │                              4. package ai-skills   │
                │                              5. GitHub Release      │
                │                              6. publish 48 crates   │
                │                                 to crates.io        │
                │                              7. publish wheels +    │
                │                                 sdist to PyPI       │
                │                                                    │
                └────────────────────────────────────────────────────┘
```

The single non-obvious link in this chain is step 7 of `version-bump.yml`:
the explicit `gh workflow run`. Without it, `release.yml` would never
fire. See [Why the trampoline exists](#why-the-trampoline-exists) below.

## Conventional Commits → bump mapping

This table is enforced by `version-bump.yml`. It scans every commit
subject + body since the last tag and picks the highest-priority
match.

| Commit type / footer | Effect |
|---|---|
| `feat:` (with or without scope) | minor bump → release |
| `fix:` / `perf:` / `revert:` (with or without scope) | patch bump → release |
| Any type with `!:` (e.g. `feat!:`, `fix(api)!:`) or a `BREAKING CHANGE:` footer | major bump → release |
| `Release-As: x.y.z` footer on any commit | exact-version bump → release |
| `build:` | skip — no bump, no release |
| `chore:` / `docs:` / `ci:` / `test:` / `refactor:` / `style:` | skip — no bump, no release |
| `chore(release): vX.Y.Z` | skip — and the entire workflow skips, since this is what the bot itself emits |

Decision priority (highest first):
1. `Release-As: x.y.z` footer
2. `BREAKING CHANGE` / `!:` → major
3. `feat:` → minor
4. `fix:` / `perf:` / `revert:` → patch
5. Everything else → skip

A `feat:` next to a `fix:` in the same range produces a minor bump
(major → minor → patch precedence). One `BREAKING CHANGE` anywhere
forces major.

### Choosing a commit type

Default to `build:` for anything that isn't actively shipping. This
is the safest type — no unintentional release. Switch types only
when you intend to publish:

* You want **a new feature available to consumers**: `feat:`
* You want **a bug fix available to consumers**: `fix:`
* You want **a performance change shipped**: `perf:`
* You want **to roll back a previous release**: `revert:`
* The change **breaks the public API**: append `!` or a
  `BREAKING CHANGE:` footer
* You want **a specific version number** (re-publish, jumping
  ahead, RC): `Release-As: x.y.z` footer

If a single commit has both shippable and non-shippable changes, split
it.

## The trampoline architecture

### Why the trampoline exists

GitHub Actions has a long-standing safety feature: **events caused by
a workflow that authenticated with the default `GITHUB_TOKEN` do not
trigger other workflows**. This prevents accidental infinite loops,
e.g. a CI workflow that pushes a commit triggering itself again.

In practice it means that when `version-bump.yml` does
`git push origin --follow-tags`, the resulting tag push is invisible
to `release.yml`'s `on: push: tags: ['v*']` trigger. Releases would
silently never run.

This bit the sibling `atomr` repo between v0.6.1 and v0.9.1: five tags
landed in git and zero of them ever ran `release.yml`, so crates.io
and PyPI stayed frozen at 0.6.0 for four versions. atomr-agents
inherits the trampoline pattern from that incident.

### How the trampoline works

After `version-bump.yml` pushes the tag, it runs:

```yaml
- name: Trigger release.yml against the new tag
  if: steps.decide.outputs.kind != 'skip' && github.event.inputs.dry_run != 'true'
  env:
    GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
    NEW_VERSION: ${{ steps.bump.outputs.version }}
  run: |
    gh workflow run release.yml \
      --ref "v${NEW_VERSION}" \
      -f dry_run=false \
      -f skip_python=false \
      -f skip_crates=false
```

A `workflow_dispatch` event from `gh workflow run` does NOT carry the
GITHUB_TOKEN-event-suppression bit, so `release.yml` runs normally —
just with `event_name: workflow_dispatch` instead of `push`. The
release.yml job conditions accept either:

```yaml
if: |
  startsWith(github.ref, 'refs/tags/v') &&
  (github.event_name == 'push' ||
   (github.event_name == 'workflow_dispatch' && github.event.inputs.dry_run != 'true'))
```

This requires `actions: write` permission on `version-bump.yml`'s job
(`contents: write` alone is insufficient).

### Alternatives considered

* **Use a Personal Access Token** to push the tag — works, but
  introduces a long-lived secret tied to a user account, with the
  rotation and ownership burden that brings.
* **Use a GitHub App token** — also works, but requires installing and
  maintaining an app.
* **`repository_dispatch` event** — works, but `workflow_dispatch` is
  simpler and more discoverable (`gh workflow run` shows up in normal
  CLI history).

The `gh workflow run` trampoline was chosen because it uses the
default `GITHUB_TOKEN` only, requires no app or PAT, and is fully
self-documenting in the workflow YAML.

## What the pipeline produces

| Artifact | Where it lands | Built by |
|---|---|---|
| 48 Rust crates (workspace publishables) | crates.io | `publish-crates` job, sequentially in dep order |
| `atomr-agents` binary, 5 platforms | GitHub Release | `build-binaries` matrix |
| 24 Python wheels (6 platforms × 4 ABIs) | PyPI | `build-wheels` matrix |
| 1 Python sdist | PyPI | `build-sdist` |
| `ai-skills.tar.gz` (plugin marketplace) | GitHub Release | `package-skills` |
| Release notes | GitHub Release | `github-release` (extracted from CHANGELOG.md `[Unreleased]` section) |

Binary platforms: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`
(native on `ubuntu-22.04-arm`), `x86_64-apple-darwin`,
`aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.

Wheel ABI: CPython 3.10, 3.11, 3.12, 3.13 (one wheel per platform per
ABI, built via maturin's `--interpreter` flag).

## Crate publish order

The `publish-crates` job walks crates strictly in dep-order with a
70s throttle between successful publishes (to respect crates.io
new-crate rate limits) and a 620s exponential-backoff retry on
`429 Too Many Requests` (up to 12 attempts per crate).

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
 46    atomr-agents-py-bindings  (PyO3 cdylib; deps on every internal crate)
 47    atomr-agents-cli          (binary; deps on harness/eval/registry/state/core)
 48    atomr-agents              (umbrella; published last)
```

A crate can publish only when every entry in its `[dependencies]` is
already on crates.io. `cargo publish` polls the index briefly between
crates so the next layer can resolve.

### Adding a new publishable crate

1. Add the crate to `[workspace.dependencies]` in the root `Cargo.toml`
   with `version = "X.Y.Z"` matching the current workspace version
   (cargo-edit's `cargo set-version --workspace` updates this
   automatically on subsequent releases).
2. In the new crate's `Cargo.toml`, declare every intra-workspace dep
   as `{ workspace = true }` — **never** as a hand-written
   `version = "X.Y.Z"` literal. Hand-written pins desync from the
   workspace version on every bump and produce a recurring class of
   release bugs.
3. Slot the crate into the **earliest** layer of `publish-crates` in
   `release.yml` whose prior layers have published all of its deps.
4. Update both `RELEASING.md` and `docs/release-{pipeline,process}.md`
   to reflect the new total count and ordering.
5. If your crate is purely internal (`xtask`, etc.), add
   `publish = false` to its `[package]` table so `cargo publish`
   rejects accidental publishes.

## Sibling workspace deps

`atomr-agents` consumes `atomr`, `atomr-infer`, and `atomr-accel`
through `[workspace.dependencies]` as **public crates.io version
pins only** — there are no `path = "../sibling/..."` links and the
release workflow never checks out sibling repos.

To bump a sibling dep:

1. Wait for the new sibling version to land on crates.io.
2. Update the version pin in this repo's root `Cargo.toml`.
3. `cargo update -p atomr-foo` (or `cargo update --workspace`).
4. Land it as a `feat:` / `fix:` so the bump-and-tag bot picks it up.

For local development against an unreleased sibling change, use
`[patch.crates-io]` in your personal `~/.cargo/config.toml` rather
than reintroducing path-deps. Path-deps to sibling repos are an
antipattern that make CI brittle and force every developer to maintain
matching side-by-side checkouts.

## PyPI publishing

`atomr-agents` ships as a single PyPI distribution (the project name is
`atomr-agents`), built by [`maturin`](https://www.maturin.rs/) from the
`crates/py-bindings` crate. Other internal crates compile into the
single `atomr_agents._native` extension module that the wheel ships.

Authentication uses [PyPI Trusted Publishing](https://docs.pypi.org/trusted-publishers/),
not an API token. The OIDC handshake between GitHub Actions and
PyPI is configured per-environment:

* Production: GitHub environment `pypi` ↔ pypi.org publisher
* Dry-run rehearsals: GitHub environment `testpypi` ↔ test.pypi.org publisher

If the publisher is misconfigured, the publish step fails with
`invalid-publisher: valid token, but no corresponding publisher`.

## Required setup (one-time)

### crates.io

1. Generate an API token at https://crates.io/me with the
   `publish-update` and `publish-new` scopes.
2. Add it as a repo secret named `CRATES_IO_TOKEN`
   (Settings → Secrets and variables → Actions).

### PyPI Trusted Publishing

For each environment (`pypi` for production, `testpypi` for
rehearsals):

1. Create the project on the relevant PyPI host (or upload one wheel
   manually first).
2. On that PyPI project: Manage → Publishing → Add a new publisher
   → GitHub.
3. Fill in:
   * Owner: `rustakka`
   * Repository: `atomr-agents`
   * Workflow name: `release.yml`
   * Environment: `pypi` (or `testpypi`)
4. Repeat for the other host.

Both environments must exist on the GitHub side too (Settings →
Environments → New environment), with the matching names.

### Workflow permissions

`version-bump.yml` needs:

```yaml
permissions:
  contents: write   # to commit, tag, and push
  actions: write    # to dispatch release.yml via `gh workflow run`
```

`release.yml` needs:

```yaml
permissions:
  contents: write   # github-release job creates the GH Release
  id-token: write   # publish-pypi job uses OIDC for Trusted Publishing
```

## Manual operations

Most of the time you don't need any of these — the trampoline handles
everything. They exist for recovery and pre-flight checks.

### Dry-run a release

```
gh workflow run release.yml -f dry_run=true
```

Runs the verify gate, builds every binary + wheel, runs `cargo publish
--dry-run` on a representative subset of foundation + leaf crates,
and uploads to TestPyPI. Skips crates.io and production PyPI. Use
this to rehearse a release after non-trivial changes to the pipeline
itself.

### Skip Python or Rust

```
gh workflow run release.yml -f dry_run=true -f skip_python=true   # cargo only
gh workflow run release.yml -f dry_run=true -f skip_crates=true   # wheels only
```

Useful when only one half of the pipeline is changing.

### Force a bump kind

The auto-bump can be overridden when manually dispatching
`version-bump.yml`:

```
gh workflow run version-bump.yml -f force=major
```

Valid values: `patch`, `minor`, `major`. Use this to ship a bump kind
the commit history wouldn't have produced (e.g. emergency major
revision after only `fix:` commits landed).

### Pin to an exact version

Append a `Release-As: x.y.z` footer to any commit body, OR dispatch
`version-bump.yml` with `-f release_as=x.y.z`. The bump tool uses
that exact version regardless of commit type. Common cases:

* Republishing after a botched release
* Skipping ahead to a planned RC or pre-release
* Pinning to a marketing version

### Cut a release without the trampoline

If the trampoline is broken or you need to ship from a tag that
predates it:

```
git tag v0.7.5 <sha>
git push origin v0.7.5
```

The `on: push: tags: ['v*']` trigger fires (because the push was made
by you, not by `GITHUB_TOKEN`), and `release.yml` runs normally.

### Restrict the publish list

Set the repo variable `ATOMR_AGENTS_PUBLISH_ALLOWLIST` to a
space-separated list of crate names. Only those crates publish on
the next run; the rest are skipped. Set it to the empty string to
skip publish entirely (useful for emergency hot-fix releases that
only need a GitHub Release artifact).

## Troubleshooting cookbook

### "five tags exist but nothing published"

Root cause: tags were created by `version-bump.yml` (using the default
`GITHUB_TOKEN`) without the trampoline step, so `release.yml` never
fired.

Recovery: cut a fresh release using a `fix:` commit (or
`Release-As: <next-version>` footer). The new pipeline takes over
and all 48 crates publish at the new version, jumping over the
unpublished tags. Crates.io enforces monotonically increasing
versions, so leapfrogging is fine and supported.

### "publish-crates failed mid-loop on crate N"

The publish loop treats `already uploaded` as success, so re-running
against the same tag is cheap — already-published crates skip in
under a second. Either:

* Re-tag the same version (`git tag -f vX.Y.Z; git push --force origin vX.Y.Z`)
  to re-fire the `on: push: tags` path.
* Or `gh workflow run release.yml --ref vX.Y.Z -f dry_run=false`.

Investigate the failed crate first — common causes are stale
intra-workspace pins (see next entry), a missing dep on crates.io
because the dep order is wrong, or a sibling-workspace version pin
pointing at a version that isn't on crates.io yet.

### "failed to select a version for the requirement `atomr-agents-foo = ^X.Y.Z`"

A crate has a hand-written `version = "X.Y.Z"` pin in its
`[dependencies]` instead of `{ workspace = true }`, AND the version
on crates.io doesn't satisfy the pin (typically because the workspace
moved on but the pin didn't).

Fix: change the dep to `{ workspace = true }` in that crate's
`Cargo.toml`. The workspace root's `[workspace.dependencies]` already
carries the version, and `cargo set-version --workspace` updates that
pin automatically on every release.

### "failed to select a version for the requirement `atomr = ^0.X.Y`"

A sibling workspace dep is pinned to a version that isn't yet on
crates.io. The sibling repo must release first; only then can the
matching pin land in `atomr-agents`.

### "PyPI invalid-publisher"

The Trusted Publisher claims (owner / repo / workflow / environment)
on the PyPI side don't match the actual run. Common causes:

* The publisher was registered with the wrong workflow name
  (`release.yml` vs `Release.yml` vs `release` — case-sensitive,
  filename only, no path).
* The GitHub environment name doesn't match (`pypi` vs `PyPI`).
* The publisher is registered on test.pypi.org but you're publishing
  to pypi.org (or vice versa).

Re-check the publisher registration; do not retry the workflow until
the registration matches.

### "PyPI 400 License-File LICENSE does not exist in distribution file"

Maturin auto-emits `License-File: LICENSE` when a LICENSE file exists
next to `pyproject.toml`. PyPI strictly validates that the file is
physically present in the sdist. Fix in `pyproject.toml`:

```toml
[tool.maturin]
include = [
  { path = "LICENSE", format = "sdist" },
]
```

### "Node.js 20 actions are deprecated" warning

Bump the affected action (typically `actions/checkout`) to a version
that ships on Node.js 24. As of late 2025 this means `@v5` or later.
Other Node.js 20 actions will surface their own deprecation warnings
as their EOL approaches.

### "auto-bump created a tag but release.yml didn't run"

Verify the trampoline step in `version-bump.yml` exists, has the
`actions: write` permission set on the job, and didn't fail silently.
Check the run log for the "Trigger release.yml against the new tag"
step.

If the step is missing entirely, the repo is on the pre-trampoline
version of `version-bump.yml`. Apply the trampoline change (see
[Why the trampoline exists](#why-the-trampoline-exists)).

### "release.yml ran but didn't publish to crates.io / PyPI"

Check the job-level `if:` guards. The publish jobs require:

* `startsWith(github.ref, 'refs/tags/v')` — workflow ran against a
  branch ref, not a tag. Re-dispatch with `--ref vX.Y.Z`.
* `github.event_name == 'push' || (workflow_dispatch && dry_run != 'true')`
  — a manual dispatch with `dry_run=true` only does the dry-run
  jobs, never the real publish.
* `skip_python != 'true'` / `skip_crates != 'true'` — one half was
  skipped intentionally; flip the input to re-run the other half.

## Audit trail of past regressions

* (Inherited from sibling `atomr` 0.6.1 → 0.9.1) Five tags created by
  `version-bump.yml` without the trampoline; none ran `release.yml`.
  Resolved by introducing the `gh workflow run` trampoline. atomr-
  agents adopts the trampoline from day one to avoid repeating this.

The pattern is the same as in atomr: the publish path is fragile
because it spans a workflow → workflow boundary AND a workflow →
external-registry boundary AND a per-crate `Cargo.toml` sync
boundary. Each of these has been hardened individually but the class
of bug is intrinsic to multi-registry publishing.

## When to update this document

* You change `version-bump.yml` or `release.yml` in any way that
  affects the operator-facing surface (new inputs, new conditions,
  new artifacts).
* You add a new publishable crate (update the layer diagram and the
  total count).
* You add a new artifact target (new wheel platform, new binary OS).
* You discover a new failure mode worth a troubleshooting entry.

The companion document `release-pipeline.md` covers internal workflow
detail (job names, matrix entries, exact build commands); update it
in tandem when those change.
