//! Integration tests for the fast Unix socket transport.
//!
//! Each test binds a fresh socket path derived from a unique ID to avoid
//! conflicts when tests run in parallel.

use async_trait::async_trait;
use messaggero::{
    serve, Agent, AgentCard, AgentError, Message, MessaggeroClient, TaskRequest, TaskResponse,
};
use std::time::Duration;
use tokio::time::sleep;

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Returns a unique socket path for the given test name.
fn socket(name: &str) -> String {
    format!("/tmp/messaggero-test-{name}.sock")
}

/// Simple echo agent used across multiple tests.
struct EchoAgent;

#[async_trait]
impl Agent for EchoAgent {
    fn card(&self) -> AgentCard {
        AgentCard::builder("echo")
            .description("Echoes the user message back")
            .build()
    }

    async fn handle_task(&self, req: TaskRequest) -> Result<TaskResponse, AgentError> {
        let text = req.message.text_content().unwrap_or("").to_string();
        Ok(TaskResponse::completed(
            &req.id,
            Message::agent(format!("echo: {text}")),
        ))
    }
}

/// Agent that always returns an error.
struct FailingAgent;

#[async_trait]
impl Agent for FailingAgent {
    fn card(&self) -> AgentCard {
        AgentCard::builder("fail").build()
    }

    async fn handle_task(&self, _req: TaskRequest) -> Result<TaskResponse, AgentError> {
        Err(AgentError::Internal("intentional failure".into()))
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Basic round-trip: send a task, receive the echoed response.
#[tokio::test]
async fn test_echo_round_trip() {
    let sock = socket("echo-round-trip");
    let sock_clone = sock.clone();

    tokio::spawn(async move {
        serve(EchoAgent)
            .fast(&sock_clone)
            .run()
            .await
            .expect("server error");
    });

    // Give the server time to bind the socket.
    sleep(Duration::from_millis(100)).await;

    let mut client = MessaggeroClient::connect_fast(&sock).await.unwrap();
    let req = TaskRequest::new(Message::user("hello"));
    let resp = client.send_task(req).await.unwrap();

    assert_eq!(
        resp.status.state,
        messaggero::TaskState::Completed,
        "expected Completed state"
    );
    let reply = resp
        .status
        .message
        .expect("response should contain a message");
    assert_eq!(reply.text_content(), Some("echo: hello"));
}

/// Task ID is preserved in the response.
#[tokio::test]
async fn test_task_id_preserved() {
    let sock = socket("task-id-preserved");
    let sock_clone = sock.clone();

    tokio::spawn(async move {
        serve(EchoAgent).fast(&sock_clone).run().await.ok();
    });

    sleep(Duration::from_millis(100)).await;

    let mut client = MessaggeroClient::connect_fast(&sock).await.unwrap();
    let req = TaskRequest::new(Message::user("ping")).with_id("custom-id-42");
    let resp = client.send_task(req).await.unwrap();

    assert_eq!(resp.id, "custom-id-42");
}

/// Multiple sequential requests on the same connection all succeed.
#[tokio::test]
async fn test_multiple_sequential_requests() {
    let sock = socket("multi-seq");
    let sock_clone = sock.clone();

    tokio::spawn(async move {
        serve(EchoAgent).fast(&sock_clone).run().await.ok();
    });

    sleep(Duration::from_millis(100)).await;

    let mut client = MessaggeroClient::connect_fast(&sock).await.unwrap();

    for i in 0..5u32 {
        let req = TaskRequest::new(Message::user(format!("msg-{i}")));
        let resp = client.send_task(req).await.unwrap();
        let text = resp
            .status
            .message
            .unwrap()
            .text_content()
            .unwrap()
            .to_string();
        assert_eq!(text, format!("echo: msg-{i}"));
    }
}

/// A server error is propagated back to the caller as a `TransportError`.
#[tokio::test]
async fn test_agent_error_propagated() {
    let sock = socket("agent-error");
    let sock_clone = sock.clone();

    tokio::spawn(async move {
        serve(FailingAgent).fast(&sock_clone).run().await.ok();
    });

    sleep(Duration::from_millis(100)).await;

    let mut client = MessaggeroClient::connect_fast(&sock).await.unwrap();
    let req = TaskRequest::new(Message::user("trigger error"));
    let result = client.send_task(req).await;

    assert!(result.is_err(), "expected an error from the failing agent");
}
