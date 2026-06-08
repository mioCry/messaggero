//! Multi-agent pipeline: messages flow through a chain of agents.
//!
//! Agent A (uppercase) -> Agent B (exclaim) -> result
//!
//! Demonstrates the router and discovery system.
//!
//! Run with: `cargo run --example multi_agent`

use messaggero::prelude::*;
use messaggero::transport::{Discovery, Router};
use messaggero::AgentEndpoint;
use std::sync::Arc;

// --- Agent that converts text to uppercase ---

struct UppercaseAgent;

#[async_trait]
impl Agent for UppercaseAgent {
    fn card(&self) -> AgentCard {
        AgentCard::builder("uppercase")
            .description("Converts text to uppercase")
            .skill("upper", "Uppercase", "Uppercases any text")
            .build()
    }

    async fn handle_task(&self, req: TaskRequest) -> Result<TaskResponse, AgentError> {
        let input = req.message.text_content().unwrap_or("");
        let output = input.to_uppercase();
        println!("[UppercaseAgent] {input} -> {output}");
        Ok(TaskResponse::completed(&req.id, Message::agent(output)))
    }
}

// --- Agent that adds exclamation marks ---

struct ExclaimAgent;

#[async_trait]
impl Agent for ExclaimAgent {
    fn card(&self) -> AgentCard {
        AgentCard::builder("exclaim")
            .description("Adds exclamation to text")
            .skill("exclaim", "Exclaim", "Adds !!! to messages")
            .build()
    }

    async fn handle_task(&self, req: TaskRequest) -> Result<TaskResponse, AgentError> {
        let input = req.message.text_content().unwrap_or("");
        let output = format!("{input}!!!");
        println!("[ExclaimAgent] {input} -> {output}");
        Ok(TaskResponse::completed(&req.id, Message::agent(output)))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("messaggero=info")
        .init();

    let upper_sock = "/tmp/messaggero_upper.sock";
    let exclaim_sock = "/tmp/messaggero_exclaim.sock";

    // Start both agents
    let upper = Arc::new(UppercaseAgent);
    let exclaim = Arc::new(ExclaimAgent);

    let s1 = tokio::spawn({
        let a = upper.clone();
        async move { messaggero::transport::fast::serve(a, upper_sock).await }
    });
    let s2 = tokio::spawn({
        let a = exclaim.clone();
        async move { messaggero::transport::fast::serve(a, exclaim_sock).await }
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Set up discovery and router
    let discovery = Discovery::new();
    discovery
        .register(upper.card(), AgentEndpoint::Fast(upper_sock.into()))
        .await;
    discovery
        .register(exclaim.card(), AgentEndpoint::Fast(exclaim_sock.into()))
        .await;

    let router = Router::new();
    discovery.populate_router(&router).await;

    println!("Registered agents: {:?}", router.agents().await);
    println!();

    // Pipeline: input -> uppercase -> exclaim -> output
    let input = "hello world";
    println!("Pipeline input: \"{input}\"");

    // Step 1: send to uppercase agent
    let req1 = TaskRequest::new(Message::user(input));
    let resp1 = router.send("uppercase", req1).await?;
    let intermediate = resp1
        .status
        .message
        .as_ref()
        .and_then(|m| m.text_content())
        .unwrap_or("");

    // Step 2: send result to exclaim agent
    let req2 = TaskRequest::new(Message::user(intermediate));
    let resp2 = router.send("exclaim", req2).await?;
    let final_output = resp2
        .status
        .message
        .as_ref()
        .and_then(|m| m.text_content())
        .unwrap_or("");

    println!("\nPipeline output: \"{final_output}\"");
    assert_eq!(final_output, "HELLO WORLD!!!");
    println!("Pipeline assertion passed!");

    // Clean up
    s1.abort();
    s2.abort();
    let _ = std::fs::remove_file(upper_sock);
    let _ = std::fs::remove_file(exclaim_sock);

    Ok(())
}
