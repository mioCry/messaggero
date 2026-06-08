//! Ping-pong example: two agents communicating over the fast binary transport.
//!
//! Run with: `cargo run --example ping_pong`

use messaggero::prelude::*;
use messaggero::MessaggeroClient;
use std::sync::Arc;

struct PongAgent;

#[async_trait]
impl Agent for PongAgent {
    fn card(&self) -> AgentCard {
        AgentCard::builder("pong")
            .description("Replies PONG to every message")
            .skill("pong", "Pong", "Responds with PONG")
            .build()
    }

    async fn handle_task(&self, req: TaskRequest) -> Result<TaskResponse, AgentError> {
        let input = req.message.text_content().unwrap_or("???");
        println!("[PongAgent] received: {input}");
        Ok(TaskResponse::completed(
            &req.id,
            Message::agent(format!("PONG: {input}")),
        ))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("messaggero=info")
        .init();

    let socket_path = "/tmp/messaggero_pong.sock";

    // Start the pong agent in the background
    let agent = Arc::new(PongAgent);
    let server_agent = agent.clone();
    let server =
        tokio::spawn(
            async move { messaggero::transport::fast::serve(server_agent, socket_path).await },
        );

    // Give the server a moment to bind
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Connect a client and send messages
    let mut client = MessaggeroClient::connect_fast(socket_path).await?;

    for i in 1..=5 {
        let request = TaskRequest::new(Message::user(format!("PING #{i}")));
        let response = client.send_task(request).await?;

        let reply = response
            .status
            .message
            .as_ref()
            .and_then(|m| m.text_content())
            .unwrap_or("(no reply)");

        println!("[Client] response: {reply}");
    }

    println!("\nAll pings completed successfully!");

    // Clean up
    server.abort();
    let _ = std::fs::remove_file(socket_path);

    Ok(())
}
