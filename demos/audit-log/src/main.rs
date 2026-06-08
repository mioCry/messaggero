//! Demo: Transport Audit Logs
//!
//! Shows how to use [`TransportLogger`] to record every request on
//! both transports (fast path Unix socket and A2A HTTP).
//!
//! What this demo shows:
//!   - Configuring `TransportLogger` with directory and file rotation
//!   - Attaching the logger to the `Router` via `.with_transport_logger()`
//!   - Direct attachment to `FastClient` via `.with_logger()`
//!   - Recording successful and failed tasks (to see "ok"/"error" status)
//!   - Automatic NDJSON file rotation (low max_entries to demonstrate it)
//!   - Reading and pretty-printing the generated audit log
//!
//! Run with:
//!   cargo run -p audit-log-demo
//!
//! Log files are written to `/tmp/messaggero-audit-demo/`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use messaggero::prelude::*;
use messaggero::transport::log::TransportLogger;
use messaggero::transport::{fast, Discovery, Router};
use messaggero::{AgentEndpoint, LoggingMiddleware};

// ---------------------------------------------------------------------------
// ANSI colors
// ---------------------------------------------------------------------------

const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[1;36m";
const GREEN: &str = "\x1b[1;32m";
const YELLOW: &str = "\x1b[1;33m";
const RED: &str = "\x1b[1;31m";
const MAGENTA: &str = "\x1b[1;35m";
const RESET: &str = "\x1b[0m";

fn sep(label: &str, color: &str) {
    println!(
        "\n{color}{BOLD}  ── {label} {}  {RESET}",
        "─".repeat(50_usize.saturating_sub(label.len()))
    );
}

// ---------------------------------------------------------------------------
// Test agents
// ---------------------------------------------------------------------------

/// Echo agent: returns received text in uppercase.
struct EchoAgent;

#[async_trait]
impl Agent for EchoAgent {
    fn card(&self) -> AgentCard {
        AgentCard::builder("echo")
            .description("Returns the message in uppercase")
            .skill("echo", "Echo", "Mirrors input in uppercase")
            .build()
    }

    async fn handle_task(&self, req: TaskRequest) -> Result<TaskResponse, AgentError> {
        let text = req.message.text_content().unwrap_or("").to_uppercase();
        Ok(TaskResponse::completed(&req.id, Message::agent(text)))
    }
}

/// Slow agent: introduces an artificial delay (simulates heavy processing).
struct SlowAgent;

#[async_trait]
impl Agent for SlowAgent {
    fn card(&self) -> AgentCard {
        AgentCard::builder("slow")
            .description("Responds after 80 ms (simulates latency)")
            .build()
    }

    async fn handle_task(&self, req: TaskRequest) -> Result<TaskResponse, AgentError> {
        tokio::time::sleep(Duration::from_millis(80)).await;
        Ok(TaskResponse::completed(
            &req.id,
            Message::agent("slow response"),
        ))
    }
}

/// Agent that always fails: produces a deterministic `AgentError`.
struct FailingAgent;

#[async_trait]
impl Agent for FailingAgent {
    fn card(&self) -> AgentCard {
        AgentCard::builder("failing")
            .description("Always fails — useful for testing error logs")
            .build()
    }

    async fn handle_task(&self, _req: TaskRequest) -> Result<TaskResponse, AgentError> {
        Err(AgentError::Internal("deterministic test error".into()))
    }
}

// ---------------------------------------------------------------------------
// Socket constants
// ---------------------------------------------------------------------------

const SOCK_ECHO: &str = "/tmp/msg_audit_echo.sock";
const SOCK_SLOW: &str = "/tmp/msg_audit_slow.sock";
const SOCK_FAILING: &str = "/tmp/msg_audit_failing.sock";

const LOG_DIR: &str = "/tmp/messaggero-audit-demo";

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("audit_log_demo=debug,messaggero=warn")
        .init();

    println!();
    println!("{BOLD}  messaggero — Demo Transport Audit Logs{RESET}");
    println!("{DIM}  Feature: transport-log  |  Log dir: {LOG_DIR}{RESET}");
    println!();

    let run_id = format!(
        "audit-log-demo-{}",
        messaggero::transport::log::now_iso8601()
    );

    // ----------------------------------------------------------------
    // 1. Configure TransportLogger
    //    max_entries=5 is intentionally low to demonstrate file rotation
    // ----------------------------------------------------------------
    sep("TransportLogger configuration", CYAN);

    let logger = TransportLogger::builder()
        .log_dir(LOG_DIR)
        .max_entries(5) // rotate every 5 lines
        .channel_capacity(256)
        .build()
        .await?;

    println!("  {GREEN}✓{RESET} Logger created → {DIM}{LOG_DIR}{RESET}");
    println!("  {DIM}File rotation every 5 entries (max_entries=5){RESET}");

    // ----------------------------------------------------------------
    // 2. Start agents
    // ----------------------------------------------------------------
    sep("Starting agents", CYAN);

    let echo_agent = Arc::new(MiddlewareStack::new(EchoAgent).with(LoggingMiddleware));
    let slow_agent = Arc::new(MiddlewareStack::new(SlowAgent).with(LoggingMiddleware));
    let failing_agent = Arc::new(MiddlewareStack::new(FailingAgent).with(LoggingMiddleware));

    tokio::spawn({
        let a = echo_agent.clone();
        async move { fast::serve(a, SOCK_ECHO).await.ok() }
    });
    tokio::spawn({
        let a = slow_agent.clone();
        async move { fast::serve(a, SOCK_SLOW).await.ok() }
    });
    tokio::spawn({
        let a = failing_agent.clone();
        async move { fast::serve(a, SOCK_FAILING).await.ok() }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    println!("  {GREEN}✓{RESET} echo, slow, failing → Unix sockets started");

    // ----------------------------------------------------------------
    // 3. Discovery + Router with logger attached
    // ----------------------------------------------------------------
    sep("Discovery and Router", CYAN);

    let discovery = Discovery::new();
    discovery
        .register(echo_agent.card(), AgentEndpoint::Fast(SOCK_ECHO.into()))
        .await;
    discovery
        .register(slow_agent.card(), AgentEndpoint::Fast(SOCK_SLOW.into()))
        .await;
    discovery
        .register(
            failing_agent.card(),
            AgentEndpoint::Fast(SOCK_FAILING.into()),
        )
        .await;

    // The router gets a clone of the logger: every send() produces a LogEntry
    let router = Router::new().with_transport_logger(logger.clone());
    discovery.populate_router(&router).await;

    let agents = router.agents().await;
    println!(
        "  {GREEN}✓{RESET} Router active  →  agents: {}",
        agents.join(", ")
    );
    println!("  {DIM}Transport: fast (Unix socket + bincode)  |  Logger attached{RESET}");

    // ----------------------------------------------------------------
    // 4. Direct demo with FastClient + logger
    //    (shows that logging works even when bypassing the Router)
    // ----------------------------------------------------------------
    sep("Test 0 — Direct FastClient with logger", MAGENTA);

    {
        let mut client = fast::FastClient::connect(SOCK_ECHO)
            .await?
            .with_logger(logger.clone());

        let req = TaskRequest::new(Message::user("direct ping")).with_session(run_id.clone());
        let result = client.send_task(req).await?;
        println!("  {GREEN}✓{RESET} FastClient → {DIM}{:?}{RESET}", result);
    }

    // ----------------------------------------------------------------
    // 5. Task series via Router
    // ----------------------------------------------------------------
    sep("Test 1-4 — Successful tasks (echo)", GREEN);

    let inputs = ["hello", "world", "audit log", "messaggero"];
    for (i, text) in inputs.iter().enumerate() {
        let req = TaskRequest::new(Message::user(*text)).with_session(run_id.clone());
        let resp = router.send("echo", req).await?;
        let out = resp
            .status
            .message
            .as_ref()
            .and_then(|m| m.text_content())
            .unwrap_or("—");
        println!(
            "  {GREEN}[{:02}]{RESET} {DIM}\"{text}\"{RESET} → {BOLD}{out}{RESET}",
            i + 1
        );
    }

    sep("Test 5-6 — Slow tasks (slow)", YELLOW);

    for i in 0..2 {
        let req = TaskRequest::new(Message::user("slow task")).with_session(run_id.clone());
        let start = std::time::Instant::now();
        let _ = router.send("slow", req).await?;
        println!(
            "  {YELLOW}[{:02}]{RESET} slow completed in {DIM}{}ms{RESET}",
            i + 1,
            start.elapsed().as_millis()
        );
    }

    sep("Test 7-8 — Failed tasks (failing)", RED);

    for i in 0..2 {
        let req = TaskRequest::new(Message::user("error test")).with_session(run_id.clone());
        match router.send("failing", req).await {
            Ok(resp) => {
                println!(
                    "  {RED}[{:02}]{RESET} unexpected: {:?}",
                    i + 1,
                    resp.status.state
                )
            }
            Err(e) => {
                println!(
                    "  {RED}[{:02}]{RESET} expected error → {DIM}{e}{RESET}",
                    i + 1
                )
            }
        }
    }

    sep("Test 9 — Non-existent agent (routing error)", RED);

    match router
        .send(
            "does-not-exist",
            TaskRequest::new(Message::user("test")).with_session(run_id.clone()),
        )
        .await
    {
        Ok(_) => println!("  {RED}✗{RESET} unexpected: no error"),
        Err(e) => println!("  {RED}✓{RESET} expected routing error → {DIM}{e}{RESET}"),
    }

    // ----------------------------------------------------------------
    // 6. Wait for async writer flush
    // ----------------------------------------------------------------
    println!("\n{DIM}  Waiting for async writer flush (200 ms)…{RESET}");
    tokio::time::sleep(Duration::from_millis(200)).await;

    // ----------------------------------------------------------------
    // 7. Read and print generated NDJSON files
    // ----------------------------------------------------------------
    sep("Audit log file contents", BOLD);

    let mut dir_entries: Vec<_> = std::fs::read_dir(LOG_DIR)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |x| x == "ndjson"))
        .collect();

    dir_entries.sort_by_key(|e| e.file_name());

    if dir_entries.is_empty() {
        println!("  {RED}✗{RESET} No files found in {LOG_DIR}");
    } else {
        println!(
            "  {GREEN}✓{RESET} Found {BOLD}{}{RESET} NDJSON file(s)\n",
            dir_entries.len()
        );

        let mut total_entries = 0usize;
        let mut ok_count = 0usize;
        let mut error_count = 0usize;
        let mut total_us: u64 = 0;
        let mut total_transport_us: u64 = 0;
        let mut transport_samples = 0usize;
        let mut ignored_without_session = 0usize;

        for de in &dir_entries {
            let fname = de.file_name();
            println!("  {BOLD}{CYAN}📄 {}{RESET}", fname.to_string_lossy());

            let content = std::fs::read_to_string(de.path())?;
            for line in content.lines() {
                if line.is_empty() {
                    continue;
                }

                let v: serde_json::Value =
                    serde_json::from_str(line).unwrap_or(serde_json::Value::Null);

                match v["session_id"].as_str() {
                    Some(session_id) if session_id == run_id => {}
                    None => {
                        ignored_without_session += 1;
                        continue;
                    }
                    Some(_) => continue,
                }

                total_entries += 1;

                let ts = v["ts"].as_str().unwrap_or("?");
                let transport = v["transport"].as_str().unwrap_or("?");
                let direction = v["direction"].as_str().unwrap_or("?");
                let task_id = v["task_id"].as_str().unwrap_or("?");
                let duration_us = v["duration_us"].as_u64().unwrap_or(0);
                let llm_us = v["llm_us"].as_u64();
                let transport_us = v["transport_us"].as_u64();
                let status = v["status"].as_str().unwrap_or("?");
                let error = v["error"].as_str();

                if status == "ok" {
                    ok_count += 1;
                } else {
                    error_count += 1;
                }
                total_us += duration_us;

                let status_color = if status == "ok" { GREEN } else { RED };
                let dir_color = if direction == "outbound" {
                    CYAN
                } else {
                    YELLOW
                };

                print!(
                    "    {DIM}{ts}{RESET}  {dir_color}{direction:<9}{RESET}  \
                     {DIM}{transport:<5}{RESET}  \
                     {status_color}{BOLD}{status}{RESET}  \
                     {DIM}total {duration_us:>6}µs{RESET}",
                );
                if let Some(t) = transport_us {
                    total_transport_us += t;
                    transport_samples += 1;
                    print!("  {DIM}overhead {t:>5}µs{RESET}");
                }
                if let Some(l) = llm_us {
                    print!("  {DIM}llm {l:>6}µs{RESET}");
                }
                print!("  {DIM}{}{RESET}", &task_id[..8.min(task_id.len())]);
                if let Some(err) = error {
                    print!("  {RED}{DIM}↳ {err}{RESET}");
                }
                println!();
            }
            println!();
        }

        // ----------------------------------------------------------------
        // 8. Statistics summary
        // ----------------------------------------------------------------
        println!("  {}", "─".repeat(60));
        println!("  {BOLD}Audit log summary{RESET}");
        println!("  {}", "─".repeat(60));
        println!(
            "  {DIM}NDJSON files generated:{RESET}   {BOLD}{}{RESET}",
            dir_entries.len()
        );
        println!("  {DIM}Current run entries:   {RESET}   {BOLD}{total_entries}{RESET}");
        println!("  {DIM}Successes (ok):        {RESET}   {GREEN}{BOLD}{ok_count}{RESET}");
        println!("  {DIM}Errors:                {RESET}   {RED}{BOLD}{error_count}{RESET}");
        if total_entries > 0 {
            println!(
                "  {DIM}Average round-trip:    {RESET}   {YELLOW}{BOLD}{:.0}µs{RESET}",
                total_us as f64 / total_entries as f64
            );
        }
        if transport_samples > 0 {
            println!(
                "  {DIM}Average overhead:      {RESET}   {CYAN}{BOLD}{:.0}µs{RESET}",
                total_transport_us as f64 / transport_samples as f64
            );
        }
        if ignored_without_session > 0 {
            println!(
                "  {DIM}Historical rows ignored without session_id:{RESET} {ignored_without_session}"
            );
        }
        println!("  {}", "─".repeat(60));
    }

    // ----------------------------------------------------------------
    // Cleanup socket
    // ----------------------------------------------------------------
    for sock in [SOCK_ECHO, SOCK_SLOW, SOCK_FAILING] {
        let _ = std::fs::remove_file(sock);
    }

    println!("\n{DIM}  Log files remain in {LOG_DIR}/{RESET}");
    println!("{GREEN}{BOLD}  Demo completed.{RESET}\n");

    Ok(())
}
