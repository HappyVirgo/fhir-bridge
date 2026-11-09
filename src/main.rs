//! Command-line interface for fhir-bridge.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use clap::{Args, Parser, Subcommand};
use fhir_bridge::deid::Deidentifier;
use fhir_bridge::fhir::{self, Bundle, ConvertOptions};
use fhir_bridge::hl7::{split_messages, Message};
use fhir_bridge::validate::{validate, Severity};
use fhir_bridge::{datetime, synth};

#[derive(Parser)]
#[command(name = "fhir-bridge", version, about = "Convert HL7 v2 messages to FHIR R4 transaction bundles")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert an HL7 v2 file to FHIR JSON (one Bundle, or an array of Bundles for multi-message files)
    Convert {
        /// Input .hl7 file
        input: PathBuf,
        /// Write JSON here instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[command(flatten)]
        options: ConvertArgs,
    },
    /// Convert every .hl7 file in a directory, writing <name>.json files to an output directory
    Batch {
        /// Directory containing .hl7 files
        dir: PathBuf,
        /// Directory for the generated JSON files (created if missing)
        #[arg(long)]
        out_dir: PathBuf,
        #[command(flatten)]
        options: ConvertArgs,
    },
    /// Check HL7 v2 files (or directories of .hl7 files) for structural and semantic problems
    Validate {
        /// Files or directories to validate
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
    },
    /// Write a deterministic corpus of synthetic HL7 messages
    Generate {
        /// Number of messages
        #[arg(long, default_value_t = 1000)]
        count: usize,
        /// Random seed
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Output file
        #[arg(short, long)]
        output: PathBuf,
    },
}

#[derive(Args)]
struct ConvertArgs {
    /// Apply Safe Harbor-style de-identification (requires --salt or FHIR_BRIDGE_SALT)
    #[arg(long)]
    deidentify: bool,
    /// Secret salt for pseudonyms and date shifting
    #[arg(long, env = "FHIR_BRIDGE_SALT", hide_env_values = true)]
    salt: Option<String>,
    /// UTC offset assumed for timestamps without one, e.g. Z, +05:30, -0800
    #[arg(long, default_value = "Z", allow_hyphen_values = true)]
    tz: String,
    /// Emit compact JSON instead of pretty-printed JSON
    #[arg(long)]
    compact: bool,
}

/// Everything needed to convert messages, resolved from the command-line flags.
struct Pipeline {
    options: ConvertOptions,
    deidentifier: Option<Deidentifier>,
    compact: bool,
}

impl Pipeline {
    fn from_args(args: ConvertArgs) -> Result<Self, String> {
        let offset =
            datetime::parse_offset(&args.tz).ok_or_else(|| format!("invalid --tz value {:?}", args.tz))?;
        let deidentifier = if args.deidentify {
            let salt = args.salt.unwrap_or_default();
            Some(Deidentifier::new(&salt).map_err(|e| format!("{e} (pass --salt or set FHIR_BRIDGE_SALT)"))?)
        } else {
            None
        };
        Ok(Self {
            options: ConvertOptions { default_offset_minutes: offset, ..ConvertOptions::default() },
            deidentifier,
            compact: args.compact,
        })
    }

    /// Converts every message in `text`; errors name the source file and message index.
    fn convert_text(&self, text: &str, source: &Path) -> Result<Vec<Bundle>, String> {
        let messages = split_messages(text);
        if messages.is_empty() {
            return Err(format!("{}: no MSH segment found", source.display()));
        }
        let mut bundles = Vec::with_capacity(messages.len());
        for (index, raw) in messages.into_iter().enumerate() {
            let at = |e: &dyn std::fmt::Display| format!("{} (message {}): {e}", source.display(), index + 1);
            let message = Message::parse(raw).map_err(|e| at(&e))?;
            let (bundle, warnings) =
                fhir::message_to_bundle(&message, &self.options, self.deidentifier.as_ref())
                    .map_err(|e| at(&e))?;
            for warning in warnings {
                eprintln!("warning: {}", at(&warning));
            }
            bundles.push(bundle);
        }
        Ok(bundles)
    }

    fn to_json(&self, bundles: &[Bundle]) -> String {
        let result = match (bundles, self.compact) {
            ([single], true) => serde_json::to_string(single),
            ([single], false) => serde_json::to_string_pretty(single),
            (many, true) => serde_json::to_string(many),
            (many, false) => serde_json::to_string_pretty(many),
        };
        result.expect("FHIR model serialization is infallible")
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Convert { input, output, options } => convert(&input, output.as_deref(), options),
        Command::Batch { dir, out_dir, options } => batch(&dir, &out_dir, options),
        Command::Validate { inputs } => validate_paths(&inputs),
        Command::Generate { count, seed, output } => generate(count, seed, &output),
    };
    result.unwrap_or_else(|message| {
        eprintln!("error: {message}");
        ExitCode::FAILURE
    })
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    fs::write(path, contents).map_err(|e| format!("{}: {e}", path.display()))
}

fn convert(input: &Path, output: Option<&Path>, args: ConvertArgs) -> Result<ExitCode, String> {
    let pipeline = Pipeline::from_args(args)?;
    let bundles = pipeline.convert_text(&read(input)?, input)?;
    let json = pipeline.to_json(&bundles);
    match output {
        Some(path) => write(path, &json)?,
        None => writeln!(std::io::stdout(), "{json}").map_err(|e| e.to_string())?,
    }
    Ok(ExitCode::SUCCESS)
}

/// The `.hl7` files directly inside `dir`, sorted by name.
fn hl7_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("hl7")))
        .collect();
    files.sort();
    Ok(files)
}

fn batch(dir: &Path, out_dir: &Path, args: ConvertArgs) -> Result<ExitCode, String> {
    let pipeline = Pipeline::from_args(args)?;
    fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let started = Instant::now();
    let (mut converted, mut failed, mut bytes) = (0usize, 0usize, 0usize);
    for file in hl7_files(dir)? {
        let text = read(&file)?;
        bytes += text.len();
        match pipeline.convert_text(&text, &file) {
            Ok(bundles) => {
                converted += bundles.len();
                let stem = file.file_stem().unwrap_or_default().to_string_lossy();
                write(&out_dir.join(format!("{stem}.json")), &pipeline.to_json(&bundles))?;
            }
            Err(message) => {
                failed += 1;
                eprintln!("error: {message}");
            }
        }
    }
    println!(
        "converted {converted} message(s); {failed} file(s) failed; {:.1} KB in {:.1} ms",
        bytes as f64 / 1024.0,
        started.elapsed().as_secs_f64() * 1000.0
    );
    Ok(if failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// Validates one file and returns `(error count, report lines)`.
fn validate_file(text: &str) -> (usize, Vec<String>) {
    let messages = split_messages(text);
    if messages.is_empty() {
        return (1, vec!["  error: no MSH segment found".to_owned()]);
    }
    let mut errors = 0;
    let mut lines = Vec::new();
    for (index, raw) in messages.into_iter().enumerate() {
        match Message::parse(raw) {
            Ok(message) => {
                for issue in validate(&message) {
                    errors += usize::from(issue.severity == Severity::Error);
                    lines.push(format!("  message {}: {issue}", index + 1));
                }
            }
            Err(e) => {
                errors += 1;
                lines.push(format!("  message {}: error: {e}", index + 1));
            }
        }
    }
    (errors, lines)
}

fn validate_paths(inputs: &[PathBuf]) -> Result<ExitCode, String> {
    let mut files = Vec::new();
    for input in inputs {
        if input.is_dir() {
            files.extend(hl7_files(input)?);
        } else {
            files.push(input.clone());
        }
    }
    let mut invalid = 0;
    for file in &files {
        let (errors, lines) = validate_file(&read(file)?);
        println!("{} {}", if errors == 0 { "OK" } else { "INVALID" }, file.display());
        for line in lines {
            println!("{line}");
        }
        invalid += usize::from(errors > 0);
    }
    println!("{} file(s) checked, {invalid} invalid", files.len());
    Ok(if invalid == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

fn generate(count: usize, seed: u64, output: &Path) -> Result<ExitCode, String> {
    let corpus = synth::messages(seed, count).join("\r");
    write(output, &corpus)?;
    println!("wrote {count} synthetic messages ({} bytes) to {}", corpus.len(), output.display());
    Ok(ExitCode::SUCCESS)
}
