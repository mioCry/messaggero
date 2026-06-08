# messaggero

[![CI](https://github.com/mioCry/messaggero/actions/workflows/ci.yml/badge.svg)](https://github.com/mioCry/messaggero/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/messaggero.svg)](https://crates.io/crates/messaggero)
[![docs.rs](https://docs.rs/messaggero/badge.svg)](https://docs.rs/messaggero)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![MSRV: 1.75](https://img.shields.io/badge/msrv-1.75-orange.svg)](https://blog.rust-lang.org/2023/12/28/Rust-1.75.0.html)

A Rust library for building high-performance multi-agent systems with a protocol
designed around two complementary transport modes: a binary fast path for
agents running on the same host, and an A2A-compatible HTTP transport for
cross-vendor interoperability.

## Motivation

Multi-agent AI systems introduce a new class of latency: the overhead of passing
tasks, messages, and results between agents. Existing Rust crates either implement
the A2A standard over HTTP/JSON-RPC (good for interoperability, slower for local
communication) or provide proprietary binary protocols (fast, but isolated from
the broader ecosystem). messaggero combines both approaches in a single library.

When two agents run on the same machine, messaggero uses a length-prefixed
bincode frame over a Unix domain socket, reducing serialization and transport
overhead to the microsecond range. When communicating with external or third-party
agents, the same API switches automatically to JSON-RPC 2.0 over HTTP, following
the Agent-to-Agent (A2A) protocol specification maintained by the Linux Foundation
with support from Google, AWS, Microsoft, and 150+ organizations.

## Design Principles

- The `Agent` trait is the only contract an implementor must satisfy. Everything
  else is opt-in through feature flags and middleware.
- The router selects the transport automatically based on how each agent endpoint
  was registered. The application code is identical regardless of whether the
  target agent is local or remote.
- Types used on the wire (`TaskRequest`, `TaskResponse`, `Message`, `Part`,
  `Artifact`, `AgentCard`) mirror the A2A v1.0 data model. An agent exposed over
  the A2A transport can be discovered and called by any compliant client.
- The middleware pipeline follows a chain-of-responsibility pattern. Logging,
  authentication, rate-limiting, and retry logic are composable layers that wrap
  any `Agent` implementation without modifying it.
- Binary serialization uses bincode exclusively on the fast path. `serde_json::Value`
  is deliberately absent from wire types to avoid `deserialize_any` incompatibility
  with non-self-describing formats.

## Transport Comparison

| | Fast Path | A2A Path |
|---|---|---|
| Wire format | bincode | JSON-RPC 2.0 |
| Transport | Unix domain socket | HTTP (axum) |
| Overhead | microseconds | milliseconds |
| Interoperability | Rust-to-Rust | Any A2A-compliant agent |
| Discovery | In-process registry | `/.well-known/agent.json` |

## Project Structure

| Path | Role |
|---|---|
| `src/core/` | Wire types, `Agent` trait, middleware pipeline, codec, JSON-RPC types |
| `src/transport/` | Fast path server/client, A2A HTTP server/client, router, discovery |
| `examples/` | Runnable examples (`ping_pong`, `multi_agent`) |
| `demos/` | Optional demos (Ollama pipeline, audit logging) — not published to crates.io |

## Quick Start

```rust
use messaggero::prelude::*;

struct EchoAgent;

#[async_trait]
impl Agent for EchoAgent {
    fn card(&self) -> AgentCard {
        AgentCard::builder("echo")
            .description("Echoes messages back")
            .skill("echo", "Echo", "Echoes any message")
            .build()
    }

    async fn handle_task(&self, req: TaskRequest) -> Result<TaskResponse, AgentError> {
        let text = req.message.text_content().unwrap_or("...");
        Ok(TaskResponse::completed(&req.id, Message::agent(format!("Echo: {text}"))))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    messaggero::serve(EchoAgent)
        .fast("/tmp/echo.sock")   // fast binary transport
        .http("127.0.0.1:3000")   // A2A-compatible HTTP transport
        .run()
        .await
}
```

## Feature Flags

| Flag | Default | Enables |
|---|---|---|
| `fast` | yes | Unix socket + bincode fast path |
| `a2a` | yes | HTTP/JSON-RPC A2A-compatible transport |
| `transport-log` | **no** | Async transport audit logger (see below) |
| `full` | no | All of the above |

## Transport Audit Logging

The optional `transport-log` feature provides a non-blocking, rotating JSON log
of every task request processed by the library. It is **disabled by default** and
has **zero overhead** when absent (all logging code is compiled out via `#[cfg]`).

### Enabling

```toml
[dependencies]
messaggero = { version = "0.1", features = ["transport-log"] }
```

No additional dependencies are required — the timestamp formatting is implemented
without third-party crates.

### What is logged

A [`LogEntry`] is written for every task request, on both the server and client side:

| Field | Description |
|---|---|
| `ts` | ISO-8601 UTC timestamp with microsecond precision |
| `transport` | `"fast"` or `"a2a"` |
| `direction` | `"inbound"` (server) or `"outbound"` (client) |
| `task_id` | Task UUID from the request/response |
| `duration_us` | Elapsed microseconds (round-trip for clients, handler time for servers) |
| `status` | `"ok"` or `"error"` |
| `error` | Error description (only present when `status == "error"`) |
| `payload_bytes` | Serialised payload size in bytes (when measurable) |

### Log file format

Files are written as [NDJSON](https://ndjson.org/) (one JSON object per line)
to the configured directory, rotating every 1 000 entries (configurable):

```text
/var/log/myapp/transport/
├── transport-2026-06-08T09-51-00.123456Z.ndjson  # 1 000 entries
├── transport-2026-06-08T09-52-01.456789Z.ndjson  # current file
```

Example lines:

```json
{"ts":"2026-06-08T09:51:00.123456Z","transport":"fast","direction":"outbound","task_id":"abc","duration_us":84,"status":"ok"}
{"ts":"2026-06-08T09:51:00.456Z","transport":"a2a","direction":"inbound","task_id":"def","duration_us":4231,"status":"error","error":"model timed out"}
```

### How it works

Logging is **fully asynchronous**: the transport hot path calls
`mpsc::Sender::try_send` (non-blocking) and a dedicated Tokio task drains the
channel and performs all file I/O. If the channel fills up, entries are
**silently dropped** — the transport latency is never affected.

### Usage

```rust
use messaggero::{TransportLogger, serve};

// Build the logger (spawns the background writer task)
let logger = TransportLogger::builder()
    .log_dir("/var/log/myapp/transport")  // required
    .max_entries(1_000)                   // default: 1 000 entries per file
    .channel_capacity(4_096)              // default: 4 096 queued entries
    .build()
    .await?;

// Server-side: logs every inbound task (both fast and A2A)
serve(MyAgent)
    .fast("/tmp/agent.sock")
    .http("127.0.0.1:3000")
    .with_transport_logger(logger.clone())
    .run()
    .await?;

// Router (client-side): logs every outbound dispatch
let router = Router::new().with_transport_logger(logger.clone());

// FastClient (client-side): logs outbound fast-path calls
let client = FastClient::connect("/tmp/agent.sock").await?
    .with_logger(logger.clone());

// MessaggeroClient (unified client): attach after connecting
let client = MessaggeroClient::connect_fast("/tmp/agent.sock")
    .await?
    .with_transport_logger(&logger);
```

## Examples

```bash
# Ping-pong between two agents via fast transport
cargo run --example ping_pong

# Multi-agent pipeline with router and discovery
cargo run --example multi_agent

# Full demo with Ollama (requires ollama + qwen3.5:4b)
cargo run -p qwen-agents
```

## Testing

```bash
# Run the full test suite (unit + integration + doc tests)
cargo test --all-features

# Run integration tests only
cargo test --all-features --test '*'
```

## Benchmarks

```bash
# Transport throughput + serialization benchmarks
cargo bench --all-features
```

Results are written to `target/criterion/` as HTML reports.

## Status

Early development. The wire format and public API are not yet stable. The crate
follows semantic versioning; breaking changes will be indicated by a major version
bump once the 1.0 milestone is reached.

Contributions, issue reports, and protocol feedback are welcome.

## License

Licensed under either of:

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.
