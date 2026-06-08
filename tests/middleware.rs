//! Integration tests for the middleware pipeline.

use async_trait::async_trait;
use messaggero::{
    serve, Agent, AgentCard, AgentError, Message, MessaggeroClient, Middleware, MiddlewareStack,
    TaskRequest, TaskResponse,
};
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::time::sleep;

fn socket(name: &str) -> String {
    format!("/tmp/messaggero-test-mw-{name}.sock")
}

// ── Test agent ───────────────────────────────────────────────────────────────

struct CountingAgent {
    count: Arc<AtomicU32>,
}

#[async_trait]
impl Agent for CountingAgent {
    fn card(&self) -> AgentCard {
        AgentCard::builder("counter").build()
    }

    async fn handle_task(&self, req: TaskRequest) -> Result<TaskResponse, AgentError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(TaskResponse::completed(&req.id, Message::agent("counted")))
    }
}

// ── Test middlewares ─────────────────────────────────────────────────────────

/// Records how many times it was invoked.
struct CountingMiddleware {
    invocations: Arc<AtomicU32>,
}

#[async_trait]
impl Middleware for CountingMiddleware {
    async fn process(
        &self,
        request: TaskRequest,
        next: &dyn Agent,
    ) -> Result<TaskResponse, AgentError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        next.handle_task(request).await
    }
}

/// Middleware that prefixes every outgoing text response.
struct PrefixMiddleware {
    prefix: String,
}

#[async_trait]
impl Middleware for PrefixMiddleware {
    async fn process(
        &self,
        request: TaskRequest,
        next: &dyn Agent,
    ) -> Result<TaskResponse, AgentError> {
        let mut resp = next.handle_task(request).await?;

        if let Some(ref mut msg) = resp.status.message {
            for part in &mut msg.parts {
                if let messaggero::Part::Text(ref mut tp) = part {
                    tp.text = format!("{}: {}", self.prefix, tp.text);
                }
            }
        }

        Ok(resp)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Middleware is called exactly once per request.
#[tokio::test]
async fn test_middleware_invoked_once() {
    let sock = socket("invoked-once");
    let invocations = Arc::new(AtomicU32::new(0));
    let inv_clone = invocations.clone();

    tokio::spawn(async move {
        let mw = CountingMiddleware {
            invocations: inv_clone,
        };
        let agent = MiddlewareStack::new(CountingAgent {
            count: Arc::new(AtomicU32::new(0)),
        })
        .with(mw);

        serve(agent).fast(&sock).run().await.ok();
    });

    sleep(Duration::from_millis(100)).await;

    let sock2 = format!("/tmp/messaggero-test-mw-{}.sock", "invoked-once");
    let mut client = MessaggeroClient::connect_fast(&sock2).await.unwrap();
    client
        .send_task(TaskRequest::new(Message::user("test")))
        .await
        .unwrap();

    assert_eq!(invocations.load(Ordering::SeqCst), 1);
}

/// Multiple middlewares are all called in order.
#[tokio::test]
async fn test_multiple_middlewares_order() {
    let sock = socket("multi-order");
    let sock2 = sock.clone();

    tokio::spawn(async move {
        let agent = MiddlewareStack::new(CountingAgent {
            count: Arc::new(AtomicU32::new(0)),
        })
        .with(PrefixMiddleware {
            prefix: "outer".into(),
        })
        .with(PrefixMiddleware {
            prefix: "inner".into(),
        });

        serve(agent).fast(&sock2).run().await.ok();
    });

    sleep(Duration::from_millis(100)).await;

    let mut client = MessaggeroClient::connect_fast(&sock).await.unwrap();
    let resp = client
        .send_task(TaskRequest::new(Message::user("x")))
        .await
        .unwrap();

    let text = resp
        .status
        .message
        .unwrap()
        .text_content()
        .unwrap()
        .to_string();
    // outer wraps inner, so result is: outer: inner: counted
    assert_eq!(text, "outer: inner: counted");
}
