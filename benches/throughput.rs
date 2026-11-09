//! Throughput of parsing, conversion and de-identification over a synthetic corpus.
//!
//! Run with `cargo bench --bench throughput`, then `cargo run --release --example bench_report`
//! to turn criterion's estimates into `bench/RESULTS.md`.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use fhir_bridge::deid::Deidentifier;
use fhir_bridge::fhir::{message_to_bundle, ConvertOptions};
use fhir_bridge::hl7::Message;
use fhir_bridge::synth;

/// Corpus parameters; `examples/bench_report.rs` regenerates the same corpus to compute rates.
const SEED: u64 = 42;
const COUNT: usize = 1000;

fn corpus_benchmarks(c: &mut Criterion) {
    let corpus = synth::messages(SEED, COUNT);
    let bytes: usize = corpus.iter().map(String::len).sum();
    let options = ConvertOptions::default();
    let deid = Deidentifier::new("benchmark-salt").unwrap();

    let mut group = c.benchmark_group("corpus");
    group.throughput(Throughput::Bytes(bytes as u64));
    group.bench_function("parse", |b| {
        b.iter(|| {
            for text in &corpus {
                black_box(Message::parse(black_box(text)).unwrap());
            }
        })
    });
    group.bench_function("parse_convert_json", |b| {
        b.iter(|| {
            for text in &corpus {
                let message = Message::parse(black_box(text)).unwrap();
                let (bundle, _) = message_to_bundle(&message, &options, None).unwrap();
                black_box(serde_json::to_vec(&bundle).unwrap());
            }
        })
    });
    group.bench_function("parse_convert_deidentify_json", |b| {
        b.iter(|| {
            for text in &corpus {
                let message = Message::parse(black_box(text)).unwrap();
                let (bundle, _) = message_to_bundle(&message, &options, Some(&deid)).unwrap();
                black_box(serde_json::to_vec(&bundle).unwrap());
            }
        })
    });
    group.finish();
}

criterion_group!(benches, corpus_benchmarks);
criterion_main!(benches);
