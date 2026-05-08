use clap::{Parser, Subcommand};
use color_eyre::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use zallet_parity_core::client::RpcClient;
use zallet_parity_core::engine::ParityEngine;
use zallet_parity_core::expected_diffs::ExpectedDiffs;
use zallet_parity_core::manifest::Manifest;
use zallet_parity_core::report::{FinalReport, RunSummary};

// ── Exit codes ───────────────────────────────────────────────────────────────

/// All methods matched or were expected diffs — no unexpected gaps.
const EXIT_CLEAN: u8 = 0;
/// At least one DIFF, MISSING, or ERROR result — investigation required.
const EXIT_DIFFS_FOUND: u8 = 1;
/// Tool-level failure (bad config, manifest parse error, I/O error).
const EXIT_TOOL_FAILURE: u8 = 2;

// ── CLI definition ────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "zallet-rpc-diff",
    author,
    version,
    about = "Compare zcashd and Zallet JSON-RPC responses method-by-method.",
    long_about = "zallet-rpc-diff runs a configurable set of JSON-RPC calls against \
        two endpoints (upstream zcashd and target Zallet), normalizes the results, \
        and classifies each method as MATCH / DIFF / EXPECTED_DIFF / MISSING / ERROR.\n\n\
        See RUNBOOK.md for full setup instructions and result interpretation."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the parity check against two live RPC endpoints.
    ///
    /// Both nodes must be on the same network and ideally hold the same wallet
    /// keys for wallet-level methods to produce meaningful comparisons.
    Run {
        /// URL of the upstream (source-of-truth) zcashd endpoint.
        ///
        /// Format: http://user:password@host:port
        /// Can also be set via the UPSTREAM_URL environment variable.
        #[arg(
            short,
            long,
            env = "UPSTREAM_URL",
            value_name = "URL",
            help = "zcashd RPC URL (source of truth)",
            long_help = "JSON-RPC URL for the upstream zcashd node.\n\
                Format: http://user:password@host:port\n\
                Example: http://rpcuser:rpcpass@127.0.0.1:8232\n\
                Can also be set via the UPSTREAM_URL environment variable."
        )]
        upstream_url: String,

        /// URL of the target Zallet endpoint under test.
        ///
        /// Format: http://user:password@host:port
        /// Can also be set via the TARGET_URL environment variable.
        #[arg(
            short,
            long,
            env = "TARGET_URL",
            value_name = "URL",
            help = "Zallet RPC URL (under test)",
            long_help = "JSON-RPC URL for the target Zallet node.\n\
                Format: http://user:password@host:port\n\
                Example: http://rpcuser:rpcpass@127.0.0.1:9067\n\
                Can also be set via the TARGET_URL environment variable."
        )]
        target_url: String,

        /// Path to the method-suite manifest (TOML).
        ///
        /// Defines which RPC methods to test and their parameters.
        /// See manifest.toml for the default suite and documentation.
        #[arg(
            short,
            long,
            default_value = "manifest.toml",
            value_name = "FILE",
            help = "Method suite manifest (default: manifest.toml)"
        )]
        manifest: PathBuf,

        /// Path to the expected-differences file (TOML).
        ///
        /// Known intentional divergences listed here are labeled EXPECTED_DIFF
        /// in the report instead of DIFF. The file is optional — if absent,
        /// all diffs are treated as unexpected.
        #[arg(
            short,
            long,
            default_value = "expected_diffs.toml",
            value_name = "FILE",
            help = "Expected-differences file (default: expected_diffs.toml, optional)"
        )]
        expected_diffs: PathBuf,

        /// Path for the output report (JSON). A Markdown report is also
        /// written alongside it with the same base name.
        #[arg(
            short,
            long,
            default_value = "reports/report.json",
            value_name = "FILE",
            help = "Output report path (default: reports/report.json); .md is also written"
        )]
        output: PathBuf,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> ExitCode {
    color_eyre::install().expect("failed to install color-eyre");
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            upstream_url,
            target_url,
            manifest,
            expected_diffs,
            output,
        } => run_parity_check(upstream_url, target_url, manifest, expected_diffs, output)
            .await
            .unwrap_or_else(|e| {
                eprintln!("\n❌ Fatal error: {:#}", e);
                eprintln!("   Run with RUST_LOG=debug for more detail.");
                ExitCode::from(EXIT_TOOL_FAILURE)
            }),
    }
}

// ── Orchestrator ──────────────────────────────────────────────────────────────

/// Top-level orchestrator for the parity check.
///
/// Delegates each phase to a focused helper so changes to one phase
/// (e.g. adding a new output format) do not ripple into unrelated code.
async fn run_parity_check(
    upstream_url: String,
    target_url: String,
    manifest_path: PathBuf,
    expected_diffs_path: PathBuf,
    output_path: PathBuf,
) -> Result<ExitCode> {
    print_header(&upstream_url, &target_url);

    let manifest = load_manifest(&manifest_path)?;
    let expected_diffs = load_expected_diffs(&expected_diffs_path)?;
    let engine = build_engine(&upstream_url, &target_url)?;

    let pb = build_progress_bar(manifest.methods.len())?;
    let results = engine.run_all(manifest.methods, &expected_diffs).await;
    pb.finish_and_clear();

    let report = FinalReport::new(results);
    write_reports(&report, &output_path)?;

    let md_path = output_path.with_extension("md");
    print_summary(&report.summary, &output_path, &md_path);

    Ok(resolve_exit_code(&report.summary))
}

// ── Phase helpers ─────────────────────────────────────────────────────────────

/// Prints the startup banner showing which endpoints will be compared.
fn print_header(upstream_url: &str, target_url: &str) {
    println!("🚀 Starting Zallet Parity Check");
    println!("   Upstream: {}", upstream_url);
    println!("   Target:   {}", target_url);
    println!();
}

/// Loads and validates the method-suite manifest.
fn load_manifest(path: &Path) -> Result<Manifest> {
    Manifest::load(path)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to load manifest '{}': {}", path.display(), e))
}

/// Loads the expected-differences configuration, falling back to an empty set
/// if the file does not exist (it is intentionally optional).
fn load_expected_diffs(path: &Path) -> Result<ExpectedDiffs> {
    if !path.exists() {
        tracing::debug!(
            "Expected-diffs file '{}' not found — proceeding without.",
            path.display()
        );
        return Ok(ExpectedDiffs::none());
    }

    let ed = ExpectedDiffs::load(path).map_err(|e| {
        color_eyre::eyre::eyre!("Failed to load expected-diffs '{}': {}", path.display(), e)
    })?;

    println!(
        "   Expected diffs: {} ({} entries)",
        path.display(),
        ed.expected.len()
    );

    Ok(ed)
}

/// Constructs the parity engine by connecting to both RPC endpoints.
fn build_engine(upstream_url: &str, target_url: &str) -> Result<ParityEngine> {
    let upstream = RpcClient::new(upstream_url).map_err(|e| {
        color_eyre::eyre::eyre!("Cannot connect to upstream '{}': {}", upstream_url, e)
    })?;

    let target = RpcClient::new(target_url)
        .map_err(|e| color_eyre::eyre::eyre!("Cannot connect to target '{}': {}", target_url, e))?;

    Ok(ParityEngine::new(upstream, target))
}

/// Creates and configures the terminal progress bar for the method run.
fn build_progress_bar(method_count: usize) -> Result<ProgressBar> {
    let multi = MultiProgress::new();
    let pb = multi.add(ProgressBar::new(method_count as u64));
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
        )?
        .progress_chars("#>-"),
    );
    pb.set_message(format!("Checking {} methods...", method_count));
    Ok(pb)
}

/// Writes both the JSON and Markdown report files to disk.
///
/// Automatically creates the parent directory if it does not exist,
/// so operators can use `--output reports/report.json` without needing
/// to create the folder manually beforehand.
fn write_reports(report: &FinalReport, json_path: &Path) -> Result<()> {
    // Ensure the parent directory exists
    if let Some(parent) = json_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            color_eyre::eyre::eyre!(
                "Failed to create output directory '{}': {}",
                parent.display(),
                e
            )
        })?;
    }

    let json_output = serde_json::to_string_pretty(report)?;
    std::fs::write(json_path, &json_output).map_err(|e| {
        color_eyre::eyre::eyre!("Failed to write report to '{}': {}", json_path.display(), e)
    })?;

    let md_path = json_path.with_extension("md");
    std::fs::write(&md_path, report.to_markdown()).map_err(|e| {
        color_eyre::eyre::eyre!(
            "Failed to write Markdown report to '{}': {}",
            md_path.display(),
            e
        )
    })?;

    Ok(())
}

/// Prints the human-readable run summary and any advisory messages to stderr.
fn print_summary(s: &RunSummary, json_path: &Path, md_path: &Path) {
    println!("✅ Parity check complete!");
    println!(
        "   {} total | ✅ {} match | ❌ {} diff | 📋 {} expected | 🔍 {} missing | ⚠️  {} error",
        s.total, s.matches, s.diffs, s.expected_diffs, s.missing, s.errors
    );
    println!("   Report: {}", json_path.display());
    println!("   Report: {}", md_path.display());

    if s.diffs > 0 {
        eprintln!();
        eprintln!(
            "⚠️  {} unexpected diff(s) found. Review '{}' for affected paths.",
            s.diffs,
            json_path.display()
        );
        eprintln!("   If a diff is intentional, add it to your expected_diffs.toml.");
    }
    if s.missing > 0 {
        eprintln!();
        eprintln!(
            "🔍 {} method(s) missing on one endpoint — Zallet may not implement them yet.",
            s.missing
        );
    }
    if s.errors > 0 {
        eprintln!();
        eprintln!(
            "💥 {} error(s) occurred. Check node health and RPC auth.",
            s.errors
        );
    }
}

/// Determines the process exit code based on what the report found.
fn resolve_exit_code(s: &RunSummary) -> ExitCode {
    if s.diffs > 0 || s.missing > 0 || s.errors > 0 {
        ExitCode::from(EXIT_DIFFS_FOUND)
    } else {
        ExitCode::from(EXIT_CLEAN)
    }
}
