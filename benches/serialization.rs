//! Benchmark: serialization speed for wire types.
//!
//! Compares JSON (serde_json) vs binary (bincode) serialization for the
//! most common wire type: `TaskRequest`.
//!
//! Run with:
//!
//! ```bash
//! cargo bench --bench serialization
//! ```

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use messaggero::{Message, TaskRequest};

fn make_request() -> TaskRequest {
    TaskRequest::new(Message::user(
        "Summarise the following document and extract the key action items.",
    ))
    .with_session("bench-session")
}

fn bench_json_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("serialization/json");
    let req = make_request();
    let json = serde_json::to_vec(&req).unwrap();
    group.throughput(Throughput::Bytes(json.len() as u64));

    group.bench_function("TaskRequest/serialize", |b| {
        b.iter_batched(
            make_request,
            |r| serde_json::to_vec(&r).unwrap(),
            BatchSize::SmallInput,
        );
    });

    group.bench_function("TaskRequest/deserialize", |b| {
        b.iter_batched(
            || json.clone(),
            |bytes| serde_json::from_slice::<TaskRequest>(&bytes).unwrap(),
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn bench_bincode_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("serialization/bincode");
    let req = make_request();
    let bin = bincode::serialize(&req).unwrap();
    group.throughput(Throughput::Bytes(bin.len() as u64));

    group.bench_function("TaskRequest/serialize", |b| {
        b.iter_batched(
            make_request,
            |r| bincode::serialize(&r).unwrap(),
            BatchSize::SmallInput,
        );
    });

    group.bench_function("TaskRequest/deserialize", |b| {
        b.iter_batched(
            || bin.clone(),
            |bytes| bincode::deserialize::<TaskRequest>(&bytes).unwrap(),
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_json_serialize, bench_bincode_serialize);
criterion_main!(benches);
