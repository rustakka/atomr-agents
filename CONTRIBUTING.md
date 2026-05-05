# Contributing to atomr-agents

Thanks for considering a contribution. atomr-agents follows the
same conventions as [atomr](https://github.com/rustakka/atomr) and
[atomr-infer](https://github.com/rustakka/atomr-infer).

## Quick start

```bash
git clone https://github.com/rustakka/atomr-agents
cd atomr-agents
cargo test --workspace
```

Sibling deps (`atomr`, `atomr-infer`, `atomr-accel`) are wired as
path dependencies in the workspace `Cargo.toml`. Clone them
alongside this repo if you want to develop against unreleased
substrate features:

```
~/source/
├── atomr/
├── atomr-infer/
├── atomr-accel/
└── atomr-agents/   ← you are here
```

## Conventional Commits

Commit subjects drive the release pipeline:

| Subject | Bump |
|---|---|
| `feat: …` | minor |
| `fix: …` | patch |
| `BREAKING CHANGE` body or `feat!:` | major |
| `chore: / docs: / ci: / test: / refactor: / style: / build:` | skip (no release) |

See [`RELEASING.md`](RELEASING.md).

## Adding a new feature

1. **Pick the right crate.** Each crate owns one concern; cross-
   cutting features land as new crates rather than blob-fattening
   existing ones. See [`docs/architecture.md`](docs/architecture.md)
   for the dep graph.
2. **Add tests.** Unit tests live alongside the implementation
   (`#[cfg(test)] mod tests`). Integration tests go under
   `tests/` per crate.
3. **Document.** If you add a public type or trait, write at least
   one rustdoc paragraph explaining intent. If you add a subsystem,
   add a `docs/<topic>.md` page and link from `docs/index.md`.
4. **Ship a skill.** New subsystems get a `SKILL.md` under
   `ai-skills/skills/atomr-agents-<topic>/`. Keep it ≤ 250 lines
   and focused on *when* to invoke / *what* to write.

## Style

- `cargo fmt` (rustfmt config in `rustfmt.toml`).
- `cargo clippy --workspace -- -D warnings`.
- atomr's idiomatic-rust principles apply by extension. See
  [`atomr/docs/idiomatic-rust.md`](https://github.com/rustakka/atomr/blob/main/docs/idiomatic-rust.md).
- Public APIs should not leak `Box<dyn Any>`. Prefer typed enums.
- Prefer adding behind a feature flag over breaking an existing
  signature.

## Tests

The workspace passes 136 tests at present. New code should keep that
green:

```bash
cargo test --workspace
```

Backend feature stubs (`sqlite` / `postgres` / `pgvector` / `qdrant`
/ `chroma` / `redis`) should compile when toggled but don't have
runtime tests in CI:

```bash
cargo check -p atomr-agents-state --features sqlite,postgres
cargo check -p atomr-agents-memory --features pgvector,qdrant,chroma
cargo check -p atomr-agents-cache --features sqlite,redis
```

## Reporting issues

File at https://github.com/rustakka/atomr-agents/issues with:

- Minimal reproduction.
- `cargo --version` and `rustc --version`.
- Workspace feature flags you're using.
- Whether the issue reproduces against the published crate or only
  the path-dep build.

## License

By contributing you agree your contributions are licensed under
Apache-2.0.
