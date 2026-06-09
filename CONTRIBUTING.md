# Contributing to Messaggero

Thank you for your interest in contributing! This document explains how to get
started and what to expect from the review process.

---

## Table of Contents

1. [Code of Conduct](#code-of-conduct)
2. [Getting Started](#getting-started)
3. [Workflow](#workflow)
4. [Coding Standards](#coding-standards)
5. [Testing](#testing)
6. [Submitting a Pull Request](#submitting-a-pull-request)
7. [Releasing](#releasing)

---

## Code of Conduct

This project follows the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct).
Please be respectful in all interactions.

---

## Getting Started

**Prerequisites**

- Rust stable ≥ 1.78 (MSRV) — install via [rustup](https://rustup.rs)
- `cargo` + standard toolchain components

```bash
# Clone the repo
git clone https://github.com/mioCry/messaggero.git
cd messaggero

# Build everything (all features)
cargo build --all-features

# Run the test suite
cargo test --all-features
```

**Optional tools** (installed automatically by CI, helpful locally):

```bash
rustup component add clippy rustfmt
cargo install cargo-nextest  # faster test runner
```

---

## Workflow

1. **Open an issue** before starting significant work. This avoids duplicate
   effort and lets maintainers give early feedback on design decisions.
2. **Fork** the repository and create a feature branch:
   ```bash
   git checkout -b feat/my-feature
   ```
3. Make your changes, following the coding standards below.
4. Open a **Draft PR** early so you can get feedback while the work is in
   progress.
5. Mark the PR as **Ready for review** when all CI checks pass.

---

## Coding Standards

### Formatting

All code must be formatted with `rustfmt` using the project's `rustfmt.toml`:

```bash
cargo fmt --all
```

### Linting

All warnings are errors in CI. Run clippy before pushing:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

### Documentation

Every public item must have a `///` doc comment. Doc comments should:

- Start with a one-line summary.
- Include a `# Examples` section with a compilable (`rust`) or `ignore`
  (`rust,ignore`) code snippet.
- Include `# Errors` / `# Panics` sections where applicable.

### Commit messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(transport): add TLS support for A2A transport
fix(codec): handle empty frames gracefully
docs(agent): add cancel example to Agent trait
```

---

## Testing

### Run all tests

```bash
cargo test --all-features
```

### Integration tests only

```bash
cargo test --all-features --test '*'
```

### Doc tests only

```bash
cargo test --all-features --doc
```

### Benchmarks

```bash
cargo bench --all-features
```

**Guidelines for new tests:**

- Unit tests live in a `#[cfg(test)]` module inside the source file.
- Integration tests live in `tests/` and test the public API surface.
- Each integration test must clean up any temporary files it creates
  (e.g. Unix sockets in `/tmp/`).
- Use unique socket/file names per test to allow parallel execution.

---

## Submitting a Pull Request

- Target the `main` branch.
- Fill in the PR template (description, motivation, testing steps).
- Ensure all CI checks pass.
- Squash fixup commits before marking as ready.

---

## Releasing

Releases are managed by maintainers:

1. Bump the version in `[workspace.package]` inside `Cargo.toml`.
2. Update `CHANGELOG.md` — move `[Unreleased]` entries to the new version.
3. Commit: `chore: release v0.x.y`
4. Tag: `git tag v0.x.y && git push --tags`
5. The `publish.yml` GitHub Actions workflow publishes the single `messaggero` crate to crates.io.
