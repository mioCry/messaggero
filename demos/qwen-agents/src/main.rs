mod agents;
mod ollama;

use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use messaggero::prelude::*;
use messaggero::transport::log::TransportLogger;
use messaggero::transport::{Discovery, Router};
use messaggero::{AgentEndpoint, Metadata};

use agents::{CriticAgent, ResearcherAgent, SummarizerAgent};

const OLLAMA_URL: &str = "http://localhost:11434";
const MODEL: &str = "qwen3.5:4b";

const SOCK_RESEARCHER: &str = "/tmp/msg_researcher.sock";
const SOCK_CRITIC: &str = "/tmp/msg_critic.sock";
const SOCK_SUMMARIZER: &str = "/tmp/msg_summarizer.sock";

const AUDIT_LOG_DIR: &str = "/tmp/messaggero-qwen-audit";

// -- ANSI colors -------------------------------------------------------
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[1;36m";
const YELLOW: &str = "\x1b[1;33m";
const GREEN: &str = "\x1b[1;32m";
const RESET: &str = "\x1b[0m";

fn sep(label: &str, color: &str) {
    println!(
        "\n{color}{}  {label}  {}{RESET}",
        "─".repeat(3),
        "─".repeat(54 - label.len().min(54))
    );
}

/// Formats a duration in a human-readable way:
/// µs if < 1 ms, decimal ms for small values, otherwise seconds.
fn fmt_dur(d: Duration) -> String {
    let us = d.as_micros();
    if us < 1_000 {
        format!("{us}µs")
    } else if us < 10_000 {
        format!("{:.2}ms", us as f64 / 1_000.0)
    } else if us < 1_000_000 {
        format!("{}ms", d.as_millis())
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}

/// Prints a timing table row with colors.
fn print_timing(step: &str, llm: Duration, transport: Duration, total: Duration) {
    println!(
        "  {DIM}⏱  {BOLD}{step}{RESET}  {DIM}│{RESET}  LLM {GREEN}{}{RESET}  {DIM}│{RESET}  \
         Transport overhead {CYAN}{}{RESET}  {DIM}│{RESET}  Round-trip {YELLOW}{}{RESET}",
        fmt_dur(llm),
        fmt_dur(transport),
        fmt_dur(total),
    );
}

fn metadata_duration(meta: &Metadata, us_key: &str, ms_key: &str) -> Duration {
    meta.get(us_key)
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_micros)
        .or_else(|| {
            meta.get(ms_key)
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_millis)
        })
        .unwrap_or(Duration::ZERO)
}

/// Extracts LLM and TTFT timings from response metadata.
fn extract_timing(resp: &TaskResponse) -> (Duration, Duration) {
    let meta = match &resp.metadata {
        Some(m) => m,
        None => return (Duration::ZERO, Duration::ZERO),
    };
    (
        metadata_duration(meta, "llm_us", "llm_ms"),
        metadata_duration(meta, "ttft_us", "ttft_ms"),
    )
}

#[derive(Default)]
struct AuditStats {
    files: usize,
    entries: usize,
    ok: usize,
    errors: usize,
    total_duration_us: u64,
    total_transport_us: u64,
    transport_samples: usize,
    ignored_without_session: usize,
}

fn format_avg_us(total_us: u64, count: usize) -> String {
    if count == 0 {
        return "n/a".to_string();
    }

    let avg_us = total_us as f64 / count as f64;
    if avg_us >= 1_000_000.0 {
        format!("{:.2}s", avg_us / 1_000_000.0)
    } else if avg_us >= 1_000.0 && avg_us < 10_000.0 {
        format!("{:.2}ms", avg_us / 1_000.0)
    } else if avg_us >= 1_000.0 {
        format!("{:.1}ms", avg_us / 1_000.0)
    } else {
        format!("{avg_us:.0}µs")
    }
}

fn read_audit_stats(session_filter: Option<&HashSet<String>>) -> AuditStats {
    let mut entries = std::fs::read_dir(AUDIT_LOG_DIR)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map_or(false, |x| x == "ndjson"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    entries.sort_by_key(|e| e.file_name());

    let mut stats = AuditStats {
        files: entries.len(),
        ..AuditStats::default()
    };

    for de in entries {
        let content = std::fs::read_to_string(de.path()).unwrap_or_default();
        for line in content.lines().filter(|l| !l.is_empty()) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };

            if let Some(filter) = session_filter {
                match v["session_id"].as_str() {
                    Some(session_id) if filter.contains(session_id) => {}
                    None => {
                        stats.ignored_without_session += 1;
                        continue;
                    }
                    Some(_) => continue,
                }
            }

            stats.entries += 1;
            if v["status"].as_str() == Some("ok") {
                stats.ok += 1;
            } else {
                stats.errors += 1;
            }
            stats.total_duration_us += v["duration_us"].as_u64().unwrap_or(0);
            if let Some(transport_us) = v["transport_us"].as_u64() {
                stats.total_transport_us += transport_us;
                stats.transport_samples += 1;
            }
        }
    }

    stats
}

fn print_audit_stats(label: &str, stats: &AuditStats) {
    println!("  {BOLD}{label}{RESET}");
    println!(
        "  {DIM}Files scanned:{RESET}       {BOLD}{}{RESET}",
        stats.files
    );
    println!(
        "  {DIM}Matching entries:{RESET}   {BOLD}{}{RESET}",
        stats.entries
    );
    println!(
        "  {DIM}OK:{RESET}                 {GREEN}{BOLD}{}{RESET}",
        stats.ok
    );
    println!(
        "  {DIM}Errors:{RESET}             {YELLOW}{BOLD}{}{RESET}",
        stats.errors
    );

    if stats.entries > 0 {
        println!(
            "  {DIM}Avg round-trip:{RESET}     {YELLOW}{BOLD}{}{RESET}  {DIM}(LLM + transport){RESET}",
            format_avg_us(stats.total_duration_us, stats.entries)
        );
    }
    if stats.transport_samples > 0 {
        println!(
            "  {DIM}Avg transport overhead:{RESET} {CYAN}{BOLD}{}{RESET}  {DIM}(derived from transport_us){RESET}",
            format_avg_us(stats.total_transport_us, stats.transport_samples)
        );
    }
    if stats.ignored_without_session > 0 {
        println!(
            "  {DIM}Ignored legacy entries without session_id:{RESET} {}",
            stats.ignored_without_session
        );
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("ollama_agents=info,messaggero=warn")
        .init();

    println!();
    println!("{BOLD}  messaggero × Qwen — Multi-Agent Demo{RESET}");
    println!("{DIM}  Model: {MODEL}  |  Ollama: {OLLAMA_URL}{RESET}");
    println!("{DIM}  Audit log: {AUDIT_LOG_DIR}{RESET}");
    println!();

    // -- Transport audit logger ------------------------------------------
    let logger = TransportLogger::builder()
        .log_dir(AUDIT_LOG_DIR)
        .channel_capacity(256)
        .build()
        .await?;

    // -- Start agents on fast transport (Unix socket + bincode) ----------
    sep("Starting agents...", DIM);

    let researcher = Arc::new(ResearcherAgent::new(OLLAMA_URL, MODEL));
    let critic = Arc::new(CriticAgent::new(OLLAMA_URL, MODEL));
    let summarizer = Arc::new(SummarizerAgent::new(OLLAMA_URL, MODEL));

    tokio::spawn({
        let a = researcher.clone();
        async move {
            messaggero::transport::fast::serve(a, SOCK_RESEARCHER)
                .await
                .ok()
        }
    });
    tokio::spawn({
        let a = critic.clone();
        async move {
            messaggero::transport::fast::serve(a, SOCK_CRITIC)
                .await
                .ok()
        }
    });
    tokio::spawn({
        let a = summarizer.clone();
        async move {
            messaggero::transport::fast::serve(a, SOCK_SUMMARIZER)
                .await
                .ok()
        }
    });

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // -- Discovery and Router --------------------------------------------
    let discovery = Discovery::new();
    discovery
        .register(
            researcher.card(),
            AgentEndpoint::Fast(SOCK_RESEARCHER.into()),
        )
        .await;
    discovery
        .register(critic.card(), AgentEndpoint::Fast(SOCK_CRITIC.into()))
        .await;
    discovery
        .register(
            summarizer.card(),
            AgentEndpoint::Fast(SOCK_SUMMARIZER.into()),
        )
        .await;

    let router = Router::new().with_transport_logger(logger.clone());
    discovery.populate_router(&router).await;

    let agents: Vec<String> = router.agents().await;
    println!("\n{BOLD}Active agents:{RESET} {}", agents.join(", "));
    println!(
        "{DIM}Transport: messaggero fast path (Unix socket + bincode)  |  Model: {MODEL}{RESET}\n"
    );

    // -- Interactive loop ------------------------------------------------
    println!("{BOLD}Type a question and press Enter. Type 'exit' to quit.{RESET}\n");

    loop {
        print!("{BOLD}{CYAN}Question>{RESET} ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let question = input.trim().to_string();

        if question.is_empty() {
            continue;
        }
        if question.to_lowercase() == "exit" {
            println!("\nGoodbye!");
            break;
        }

        let conversation_id = uuid::Uuid::new_v4().to_string();
        let pipeline_start = Instant::now();

        // ----------------------------------------------------------------
        // Step 1 — Researcher
        // ----------------------------------------------------------------
        sep("Step 1 — Researcher: answering the question", "\x1b[34m");

        let step1_start = Instant::now();
        let req1 = TaskRequest::new(Message::user(&question)).with_session(conversation_id.clone());
        let resp1 = router.send("researcher", req1).await?;
        let step1_total = step1_start.elapsed();

        let (llm1, ttft1) = extract_timing(&resp1);
        let transport1 = step1_total.saturating_sub(llm1);

        print_timing("Researcher", llm1, transport1, step1_total);
        println!("{DIM}  (first token in {}){RESET}", fmt_dur(ttft1));

        let research = resp1
            .status
            .message
            .as_ref()
            .and_then(|m| m.text_content())
            .unwrap_or("")
            .to_string();

        // ----------------------------------------------------------------
        // Step 2 — Critic
        // ----------------------------------------------------------------
        sep("Step 2 — Critic: reviewing and improving", "\x1b[33m");

        let step2_start = Instant::now();
        let req2 = TaskRequest::new(Message::user(&research)).with_session(conversation_id.clone());
        let resp2 = router.send("critic", req2).await?;
        let step2_total = step2_start.elapsed();

        let (llm2, ttft2) = extract_timing(&resp2);
        let transport2 = step2_total.saturating_sub(llm2);

        print_timing("Critic", llm2, transport2, step2_total);
        println!("{DIM}  (first token in {}){RESET}", fmt_dur(ttft2));

        let critique = resp2
            .status
            .message
            .as_ref()
            .and_then(|m| m.text_content())
            .unwrap_or("")
            .to_string();

        // Extract only the IMPROVED: section if present.
        let improved = if let Some(pos) = critique.find("IMPROVED:") {
            critique[pos + "IMPROVED:".len()..].trim().to_string()
        } else {
            critique.clone()
        };

        // ----------------------------------------------------------------
        // Step 3 — Summarizer
        // ----------------------------------------------------------------
        sep("Step 3 — Summarizer: final summary", "\x1b[32m");

        let step3_start = Instant::now();
        let req3 = TaskRequest::new(Message::user(&improved)).with_session(conversation_id.clone());
        let resp3 = router.send("summarizer", req3).await?;
        let step3_total = step3_start.elapsed();

        let (llm3, ttft3) = extract_timing(&resp3);
        let transport3 = step3_total.saturating_sub(llm3);

        print_timing("Summarizer", llm3, transport3, step3_total);
        println!("{DIM}  (first token in {}){RESET}", fmt_dur(ttft3));

        // ----------------------------------------------------------------
        // Pipeline summary
        // ----------------------------------------------------------------
        let pipeline_total = pipeline_start.elapsed();
        let llm_total = llm1 + llm2 + llm3;
        let transport_total = transport1 + transport2 + transport3;

        println!();
        println!("  {}", "─".repeat(58));
        println!("  {BOLD}Pipeline summary{RESET}  ({DIM}3 agents × 3 hops via messaggero{RESET})");
        println!("  {}", "─".repeat(58));
        println!(
            "  {DIM}Total LLM:       {RESET}{GREEN}{BOLD}{}{RESET}",
            fmt_dur(llm_total)
        );
        println!(
            "  {DIM}Transport overhead: {RESET}{CYAN}{BOLD}{}{RESET}  {DIM}(estimated, messaggero fast path){RESET}",
            fmt_dur(transport_total)
        );
        println!(
            "  {DIM}Overhead %:      {RESET}{DIM}{:.2}%{RESET}",
            transport_total.as_secs_f64() / pipeline_total.as_secs_f64() * 100.0
        );
        println!(
            "  {DIM}Pipeline total:  {RESET}{YELLOW}{BOLD}{}{RESET}",
            fmt_dur(pipeline_total)
        );
        println!("  {}", "─".repeat(58));
        println!();

        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut one_conversation = HashSet::new();
        one_conversation.insert(conversation_id);
        sep("Audit log summary for this conversation", DIM);
        let stats = read_audit_stats(Some(&one_conversation));
        print_audit_stats("Conversation audit", &stats);
        println!();
    }

    // -- Cleanup socket --------------------------------------------------
    for sock in [SOCK_RESEARCHER, SOCK_CRITIC, SOCK_SUMMARIZER] {
        let _ = std::fs::remove_file(sock);
    }

    Ok(())
}
