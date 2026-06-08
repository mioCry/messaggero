//! Benchmark: throughput of the fast Unix socket transport.
//!
//! Measures how many task round-trips per second the fast path sustains on
//! the local machine.  Run with:
//!
//! ```bash
//! cargo bench --bench fast_transport
//! ```

use async_trait::async_trait;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use messaggero::{
    serve, Agent, AgentCard, AgentError, Message, MessaggeroClient, TaskRequest, TaskResponse,
};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::time::sleep;

// ── Minimal echo agent ───────────────────────────────────────────────────────

struct EchoAgent;

#[async_trait]
impl Agent for EchoAgent {
    fn card(&self) -> AgentCard {
        AgentCard::builder("bench-echo").build()
    }

    async fn handle_task(&self, req: TaskRequest) -> Result<TaskResponse, AgentError> {
        Ok(TaskResponse::completed(&req.id, Message::agent("pong")))
    }
}

// ── Bench: single-client sequential round-trips ───────────────────────────────

fn bench_round_trip(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let sock = "/tmp/messaggero-bench-fast.sock".to_string();
    let sock_srv = sock.clone();

    // Start the server in the background.
    rt.spawn(async move {
        serve(EchoAgent).fast(&sock_srv).run().await.ok();
    });

    // Give it time to bind.
    rt.block_on(sleep(Duration::from_millis(200)));

    // Connect once and reuse the connection across all iterations.
    let mut client = rt
        .block_on(MessaggeroClient::connect_fast(&sock))
        .expect("connect failed");

    let mut group = c.benchmark_group("fast_transport");
    group.throughput(Throughput::Elements(1));
    group.bench_function("sequential_round_trip", |b| {
        b.iter(|| {
            let req = TaskRequest::new(Message::user("ping"));
            rt.block_on(client.send_task(req))
                .expect("send_task failed");
        });
    });
    group.finish();
}

// ── Bench: message construction (no I/O) ─────────────────────────────────────

fn bench_message_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("types");

    group.bench_function("TaskRequest::new", |b| {
        b.iter_batched(
            || (),
            |_| TaskRequest::new(Message::user("benchmark payload")),
            BatchSize::SmallInput,
        );
    });

    group.bench_function("TaskResponse::completed", |b| {
        b.iter_batched(
            || (),
            |_| TaskResponse::completed("bench-id", Message::agent("result")),
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(10))
        .warm_up_time(Duration::from_secs(3));
    targets = bench_round_trip, bench_message_construction
}
criterion_main!(benches);
