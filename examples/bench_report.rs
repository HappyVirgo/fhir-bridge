//! Summarises criterion's results for `benches/throughput.rs` into `bench/RESULTS.md`.
//!
//! Usage: `cargo bench --bench throughput && cargo run --release --example bench_report`

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use fhir_bridge::{datetime, synth};
use serde_json::Value;

const SEED: u64 = 42;
const COUNT: usize = 1000;
const BENCHES: [(&str, &str); 3] = [
    ("parse", "Parse only"),
    ("parse_convert_json", "Parse + convert + serialize JSON"),
    ("parse_convert_deidentify_json", "Parse + convert + de-identify + serialize JSON"),
];

/// Reads the mean and its 95% confidence interval (nanoseconds per iteration) from criterion's
/// `estimates.json`.
fn estimate(dir: &Path) -> Result<(f64, f64, f64), String> {
    let path = dir.join("new").join("estimates.json");
    let text = fs::read_to_string(&path)
        .map_err(|e| format!("{}: {e} (run `cargo bench` first)", path.display()))?;
    let json: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mean = &json["mean"];
    let number = |v: &Value| v.as_f64().ok_or_else(|| format!("{}: missing mean estimate", path.display()));
    Ok((
        number(&mean["point_estimate"])?,
        number(&mean["confidence_interval"]["lower_bound"])?,
        number(&mean["confidence_interval"]["upper_bound"])?,
    ))
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn cpu_name() -> String {
    let linux = fs::read_to_string("/proc/cpuinfo").ok().and_then(|info| {
        info.lines()
            .find(|l| l.starts_with("model name"))
            .and_then(|l| l.split(':').nth(1))
            .map(|s| s.trim().to_owned())
    });
    let windows = || {
        let key = r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0";
        let out = command_output("reg", &["query", key, "/v", "ProcessorNameString"])?;
        out.lines()
            .find(|l| l.contains("ProcessorNameString"))
            .and_then(|l| l.split("REG_SZ").nth(1))
            .map(|s| s.trim().to_owned())
    };
    linux.or_else(windows).unwrap_or_else(|| "unknown CPU".to_owned())
}

fn rustc_version() -> String {
    let Some(info) = command_output("rustc", &["-vV"]) else {
        return "unknown rustc".to_owned();
    };
    let field = |key: &str| info.lines().find_map(|l| l.strip_prefix(key)).unwrap_or("?").to_owned();
    format!("rustc {} ({})", field("release: "), field("host: "))
}

fn main() -> Result<(), String> {
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| PathBuf::from("target"), PathBuf::from);
    let criterion_dir = target.join("criterion").join("corpus");
    let corpus = synth::messages(SEED, COUNT);
    let bytes: usize = corpus.iter().map(String::len).sum();

    let days = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs() / 86_400;
    let (y, m, d) = datetime::civil_from_days(days as i64);

    let mut table = String::from(
        "| Benchmark | Mean time / corpus | 95% CI of mean | Messages/sec | MB/s |\n\
         |---|---:|---:|---:|---:|\n",
    );
    for (id, label) in BENCHES {
        let (mean_ns, low_ns, high_ns) = estimate(&criterion_dir.join(id))?;
        let seconds = mean_ns / 1e9;
        table.push_str(&format!(
            "| {label} | {:.2} ms | {:.2} - {:.2} ms | {:.0} | {:.1} |\n",
            mean_ns / 1e6,
            low_ns / 1e6,
            high_ns / 1e6,
            COUNT as f64 / seconds,
            bytes as f64 / seconds / 1e6
        ));
    }

    let report = format!(
        "# Benchmark results\n\n\
         Measured {y:04}-{m:02}-{d:02} with `cargo bench --bench throughput` followed by \
         `cargo run --release --example bench_report`.\n\n\
         - **Corpus:** {COUNT} synthetic messages from `fhir_bridge::synth::messages(seed = {SEED})` \
         (mix of ADT^A01/A04/A08 and ORU^R01 with 3-8 OBX results each), {bytes} bytes total. \
         All data is fabricated.\n\
         - **Machine:** {cpu}, {os}/{arch}, {rustc}\n\
         - **Method:** criterion 0.5, 100 samples per benchmark; one iteration processes the whole \
         corpus. Messages/sec and MB/s (10^6 bytes) are derived from the mean. Default `release` \
         profile (no LTO).\n\n\
         {table}",
        cpu = cpu_name(),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        rustc = rustc_version(),
    );
    fs::create_dir_all("bench").map_err(|e| e.to_string())?;
    fs::write("bench/RESULTS.md", &report).map_err(|e| e.to_string())?;
    print!("{report}");
    Ok(())
}
