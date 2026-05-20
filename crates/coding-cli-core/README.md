# atomr-agents-coding-cli-core

Uniform contract for the coding-cli harness — wraps local AI coding CLIs
(Claude Code, Codex, Antigravity, ...) behind a single Rust surface.

This crate intentionally carries only data types and the integration
seam (`CliVendor` trait); the harness, isolator, vendor adapters, and
web companion live in sibling crates:

| Crate                                  | Role                                      |
| -------------------------------------- | ----------------------------------------- |
| `atomr-agents-coding-cli-core`         | this crate — shared types + vendor trait  |
| `atomr-agents-coding-cli-isolator`     | `Isolator` trait + Local + Docker impls   |
| `atomr-agents-coding-cli-vendor-claude`| Claude Code adapter                       |
| `atomr-agents-coding-cli-vendor-codex` | Codex CLI adapter                         |
| `atomr-agents-coding-cli-vendor-antigravity`| Antigravity CLI (`agy`) adapter      |
| `atomr-agents-coding-cli-harness`      | the harness (Callable + broadcast events) |
| `atomr-agents-coding-cli-harness-web`  | Axum + embedded SPA companion             |
