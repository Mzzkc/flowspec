//! Flowspec CLI — primary human and agent interface.
//!
//! stdout is EXCLUSIVELY for structured output (manifest, diagnostics).
//! All logging goes to stderr via tracing. This makes output pipe-safe:
//! `flowspec analyze . | other-tool`
//!
//! This binary is a thin shell: it parses CLI arguments via Clap, converts
//! them to library types, and delegates to `flowspec::commands` for all logic.
//! All testable logic lives in the library crate for coverage.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use flowspec::{commands, FlowspecError, OutputFormat};

/// Static code analyzer that traces the flow of all data in a codebase.
#[derive(Parser)]
#[command(
    name = "flowspec",
    version,
    about = "Static code analyzer for data flow tracing"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Write output to file instead of stdout.
    #[arg(short, long, global = true)]
    output: Option<PathBuf>,

    /// Output format.
    #[arg(short, long, global = true, default_value = "yaml")]
    format: Format,

    /// Verbose logging (tracing output to stderr).
    #[arg(short, long, global = true, conflicts_with = "quiet")]
    verbose: bool,

    /// Suppress all non-error output except the result.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,

    /// Path to .flowspec/config.yaml.
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Full analysis — parse, build graph, produce manifest.
    Analyze {
        /// Project root to analyze.
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Force full re-analysis (ignore cache).
        #[arg(long)]
        full: bool,

        /// Use cached graph for incremental analysis.
        #[arg(long, conflicts_with = "full")]
        incremental: bool,

        /// Restrict analysis to specific language(s).
        #[arg(short, long)]
        language: Vec<String>,

        /// Filter to specific diagnostic patterns (comma-separated).
        /// Valid patterns: isolated_cluster, data_dead_end, phantom_dependency,
        /// orphaned_impl, circular_dependency, missing_reexport, contract_mismatch,
        /// stale_reference, layer_violation, duplication, partial_wiring,
        /// asymmetric_handling, incomplete_migration.
        #[arg(long, value_delimiter = ',')]
        checks: Vec<String>,

        /// Minimum severity to report (critical, warning, info).
        #[arg(long)]
        severity: Option<SeverityArg>,

        /// Minimum confidence to report (high, moderate, low).
        #[arg(long)]
        confidence: Option<ConfidenceArg>,
    },

    /// Run diagnostics only — output structural issues found.
    Diagnose {
        /// Project root to analyze.
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Filter to specific diagnostic patterns (comma-separated).
        #[arg(long, value_delimiter = ',')]
        checks: Vec<String>,

        /// Minimum severity to report.
        #[arg(long)]
        severity: Option<SeverityArg>,

        /// Minimum confidence to report.
        #[arg(long)]
        confidence: Option<ConfidenceArg>,

        /// Restrict analysis to specific language(s).
        #[arg(short, long)]
        language: Vec<String>,
    },

    /// Trace a single symbol's complete flow through the codebase.
    Trace {
        /// Project root.
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Symbol to trace (module::name format, or partial match).
        #[arg(short, long)]
        symbol: String,

        /// Restrict analysis to specific language(s).
        #[arg(short, long)]
        language: Vec<String>,

        /// Maximum traversal depth for flow tracing.
        #[arg(long, short = 'd', default_value = "10")]
        depth: usize,

        /// Trace direction: forward (callees), backward (callers), or both.
        #[arg(long, default_value = "forward")]
        direction: TraceDirection,
    },

    /// Compare two manifests — show structural changes (not yet implemented).
    Diff {
        /// Path to older manifest.
        old: PathBuf,
        /// Path to newer manifest.
        new: PathBuf,
    },

    /// Create .flowspec/config.yaml for a project (not yet implemented).
    Init {
        /// Project root.
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Watch mode — re-analyze on file changes (not yet implemented).
    Watch {
        /// Project root.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Clone, ValueEnum)]
enum Format {
    Yaml,
    Json,
    Sarif,
    Summary,
}

impl Format {
    fn to_output_format(&self) -> OutputFormat {
        match self {
            Format::Yaml => OutputFormat::Yaml,
            Format::Json => OutputFormat::Json,
            Format::Sarif => OutputFormat::Sarif,
            Format::Summary => OutputFormat::Summary,
        }
    }
}

#[derive(Clone, ValueEnum)]
enum SeverityArg {
    Critical,
    Warning,
    Info,
}

#[derive(Clone, ValueEnum)]
enum ConfidenceArg {
    High,
    Moderate,
    Low,
}

/// Trace direction for the `trace` command.
#[derive(Clone, ValueEnum)]
enum TraceDirection {
    /// Trace callees (forward data flow).
    Forward,
    /// Trace callers (backward data flow).
    Backward,
    /// Trace both directions (forward + backward union).
    Both,
}

impl TraceDirection {
    fn to_lib(&self) -> commands::TraceDirection {
        match self {
            TraceDirection::Forward => commands::TraceDirection::Forward,
            TraceDirection::Backward => commands::TraceDirection::Backward,
            TraceDirection::Both => commands::TraceDirection::Both,
        }
    }
}

impl SeverityArg {
    fn to_severity(&self) -> flowspec::Severity {
        match self {
            SeverityArg::Critical => flowspec::Severity::Critical,
            SeverityArg::Warning => flowspec::Severity::Warning,
            SeverityArg::Info => flowspec::Severity::Info,
        }
    }
}

impl ConfidenceArg {
    fn to_confidence(&self) -> flowspec::Confidence {
        match self {
            ConfidenceArg::High => flowspec::Confidence::High,
            ConfidenceArg::Moderate => flowspec::Confidence::Moderate,
            ConfidenceArg::Low => flowspec::Confidence::Low,
        }
    }
}

fn main() -> ExitCode {
    // Use try_parse to intercept clap errors and control exit codes.
    // Clap uses exit code 2 for usage errors, but flowspec reserves exit code 2
    // for "success with findings" — the CI gate contract. All errors use exit 1.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            // --help and --version are "errors" in clap but should exit 0
            if e.kind() == clap::error::ErrorKind::DisplayHelp
                || e.kind() == clap::error::ErrorKind::DisplayVersion
            {
                return ExitCode::from(0);
            }
            return ExitCode::from(1);
        }
    };

    // Set up tracing subscriber writing to stderr
    setup_tracing(cli.verbose, cli.quiet);

    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            tracing::error!("{}", e);
            ExitCode::from(1)
        }
    }
}

fn setup_tracing(verbose: bool, quiet: bool) {
    use tracing_subscriber::fmt;
    use tracing_subscriber::EnvFilter;

    let filter = if verbose {
        EnvFilter::new("debug")
    } else if quiet {
        EnvFilter::new("error")
    } else {
        EnvFilter::new("warn")
    };

    fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn run(cli: Cli) -> Result<ExitCode, FlowspecError> {
    let format = cli.format.to_output_format();
    let output_path = cli.output.as_deref();
    let config_path = cli.config.as_deref();

    let exit_code = match cli.command {
        Commands::Analyze {
            path,
            language,
            checks,
            severity,
            confidence,
            ..
        } => commands::run_analyze(
            &path,
            &language,
            format,
            output_path,
            config_path,
            &checks,
            severity.map(|s| s.to_severity()),
            confidence.map(|c| c.to_confidence()),
        )?,
        Commands::Diagnose {
            path,
            checks,
            severity,
            confidence,
            language,
        } => commands::run_diagnose(
            &path,
            &language,
            &checks,
            severity.map(|s| s.to_severity()),
            confidence.map(|c| c.to_confidence()),
            format,
            output_path,
            config_path,
        )?,
        Commands::Trace {
            path,
            symbol,
            language,
            depth,
            direction,
        } => commands::run_trace(
            &path,
            &symbol,
            &language,
            depth,
            direction.to_lib(),
            format,
            output_path,
            config_path,
        )?,
        Commands::Diff { .. } => {
            return Err(FlowspecError::CommandNotImplemented {
                command: "diff".to_string(),
                suggestion: "use flowspec analyze to compare projects manually; diff is planned for a future release".to_string(),
            })
        }
        Commands::Init { .. } => {
            return Err(FlowspecError::CommandNotImplemented {
                command: "init".to_string(),
                suggestion: "create .flowspec/config.yaml manually; init is planned for a future release".to_string(),
            })
        }
        Commands::Watch { .. } => {
            return Err(FlowspecError::CommandNotImplemented {
                command: "watch".to_string(),
                suggestion: "use flowspec analyze in a file-watcher loop; watch is planned for a future release".to_string(),
            })
        }
    };

    Ok(ExitCode::from(exit_code))
}
