# Benchmark results

Measured 2026-11-06 with `cargo bench --bench throughput` followed by `cargo run --release --example bench_report`.

- **Corpus:** 1000 synthetic messages from `fhir_bridge::synth::messages(seed = 42)` (mix of ADT^A01/A04/A08 and ORU^R01 with 3-8 OBX results each), 607981 bytes total. All data is fabricated.
- **Machine:** 12th Gen Intel(R) Core(TM) i7-12700, windows/x86_64, rustc 1.99.1 (x86_64-pc-windows-gnu)
- **Method:** criterion 0.5, 100 samples per benchmark; one iteration processes the whole corpus. Messages/sec and MB/s (10^6 bytes) are derived from the mean. Default `release` profile (no LTO).

| Benchmark | Mean time / corpus | 95% CI of mean | Messages/sec | MB/s |
|---|---:|---:|---:|---:|
| Parse only | 2.23 ms | 2.20 - 2.26 ms | 448408 | 272.6 |
| Parse + convert + serialize JSON | 18.15 ms | 17.96 - 18.35 ms | 55103 | 33.5 |
| Parse + convert + de-identify + serialize JSON | 18.84 ms | 18.60 - 19.12 ms | 53065 | 32.3 |
