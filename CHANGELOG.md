# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Changed
- Consolidated workspace into a single publishable crate (`messaggero`); internal modules live under `src/core/` and `src/transport/`.
- crates.io publish workflow now publishes one crate only.

### Removed
- Separate workspace crates `messaggero-core`, `messaggero-transport`, and `messaggero-macros` (code merged into `messaggero`; use `AgentCard::builder()` instead of `#[derive(AgentCard)]`).

### Added
- GitHub Actions CI workflow (`ci.yml`): fmt, clippy, tests, rustdoc, MSRV check.
- GitHub Actions publish workflow (`publish.yml`): automated crates.io release on version tags.
- Integration test suite (`tests/`) covering fast transport round-trips, middleware pipelines,
  and wire-type serialization.
- `rustfmt.toml` for consistent code style across the workspace.
- `CONTRIBUTING.md` with contribution guidelines.
- `CHANGELOG.md` (this file).
- `#[deny(missing_docs, clippy::all)]` in the umbrella crate.
- `#[derive(DeriveAgentCard)]` re-exported via `messaggero::prelude` so that users
  never need to name `messaggero-macros` directly.
- Full `rustdoc` coverage on all public types: `AgentCard`, `Agent`, `Middleware`,
  `MiddlewareStack`, `LoggingMiddleware`, `Message`, `Part`, `TaskRequest`,
  `TaskResponse`, `TaskState`, `TaskStatus`, `Task`, `Metadata`.
- Criterion benchmark skeleton in `benches/`.

---

## [0.1.0] — 2026-04-23

### Added
- Initial release.
- `Agent` trait with `handle_task` / `handle_cancel` / `card`.
- `Middleware` trait and `MiddlewareStack` chain-of-responsibility implementation.
- `LoggingMiddleware` built-in.
- Wire types: `AgentCard`, `AgentSkill`, `AgentCapabilities`, `Task`, `TaskRequest`,
  `TaskResponse`, `TaskStatus`, `TaskState`, `Message`, `Part`, `Artifact`, `Metadata`.
- Fast binary transport over Unix domain sockets (bincode + length-prefix framing).
- A2A-compatible HTTP transport (axum server + reqwest client, JSON-RPC 2.0).
- In-process `Discovery` registry and `Router`.
- `ServerBuilder` (`serve(agent).fast(...).http(...).run()`).
- `MessaggeroClient` unified client with `connect_fast` / `connect_http`.
- `#[derive(AgentCard)]` procedural macro.
- Feature flags: `fast`, `a2a`, `full`.
- Examples: `ping_pong`, `multi_agent`.
- Demo: `qwen-agents` (Ollama multi-agent pipeline).
- Dual MIT / Apache-2.0 license.

[Unreleased]: https://github.com/mioCry/messaggero/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/mioCry/messaggero/releases/tag/v0.1.0
