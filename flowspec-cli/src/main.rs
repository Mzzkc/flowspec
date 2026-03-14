//! Flowspec CLI — primary human and agent interface.
//!
//! stdout is EXCLUSIVELY for structured output (manifest, diagnostics).
//! All logging goes to stderr via tracing. This makes output pipe-safe:
//! `flowspec analyze . | other-tool`

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use flowspec::{
    Config, FlowspecError, JsonFormatter, OutputFormatter, SarifFormatter, YamlFormatter,
};

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
    /// Trace callers (backward data flow — not yet implemented).
    Backward,
    /// Trace both directions (not yet implemented).
    Both,
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
    match cli.command {
        Commands::Analyze { path, language, .. } => run_analyze(
            &path,
            &language,
            &cli.format,
            cli.output.as_deref(),
            cli.config.as_deref(),
        ),
        Commands::Diagnose {
            path,
            checks,
            severity,
            confidence,
            language,
        } => run_diagnose(
            &path,
            &language,
            &checks,
            severity.as_ref(),
            confidence.as_ref(),
            &cli.format,
            cli.output.as_deref(),
            cli.config.as_deref(),
        ),
        Commands::Trace {
            path,
            symbol,
            language,
            depth,
            direction,
        } => run_trace(
            &path,
            &symbol,
            &language,
            depth,
            &direction,
            &cli.format,
            cli.output.as_deref(),
            cli.config.as_deref(),
        ),
        Commands::Diff { .. } => Err(FlowspecError::CommandNotImplemented {
            command: "diff".to_string(),
        }),
        Commands::Init { .. } => Err(FlowspecError::CommandNotImplemented {
            command: "init".to_string(),
        }),
        Commands::Watch { .. } => Err(FlowspecError::CommandNotImplemented {
            command: "watch".to_string(),
        }),
    }
}

fn run_analyze(
    path: &PathBuf,
    languages: &[String],
    format: &Format,
    output_path: Option<&std::path::Path>,
    config_path: Option<&std::path::Path>,
) -> Result<ExitCode, FlowspecError> {
    if !matches!(format, Format::Yaml | Format::Json | Format::Sarif) {
        return Err(FlowspecError::FormatNotImplemented {
            format: format_name(format).to_string(),
        });
    }

    // Validate path
    if path.as_os_str().is_empty() {
        return Err(FlowspecError::EmptyPath);
    }

    let canonical = resolve_path(path)?;
    let config = Config::load(&canonical, config_path)?;

    // Validate languages before analysis
    for lang in languages {
        validate_language(lang)?;
    }

    // Normalize language aliases (e.g., "ts" → "typescript")
    let normalized = normalize_languages(languages);

    tracing::info!("Analyzing project at {}", canonical.display());

    let result = flowspec::analyze(&canonical, &config, &normalized)?;

    let output = format_with(format, |f| f.format_manifest(&result.manifest))?;

    write_output(&output, output_path)?;

    // Exit code: 2 if critical diagnostics, 0 otherwise
    if result.has_critical {
        Ok(ExitCode::from(2))
    } else {
        Ok(ExitCode::from(0))
    }
}

#[allow(clippy::too_many_arguments)]
fn run_diagnose(
    path: &PathBuf,
    languages: &[String],
    checks: &[String],
    severity: Option<&SeverityArg>,
    confidence: Option<&ConfidenceArg>,
    format: &Format,
    output_path: Option<&std::path::Path>,
    config_path: Option<&std::path::Path>,
) -> Result<ExitCode, FlowspecError> {
    if !matches!(format, Format::Yaml | Format::Json | Format::Sarif) {
        return Err(FlowspecError::FormatNotImplemented {
            format: format_name(format).to_string(),
        });
    }

    if path.as_os_str().is_empty() {
        return Err(FlowspecError::EmptyPath);
    }

    let canonical = resolve_path(path)?;
    let config = Config::load(&canonical, config_path)?;

    // Validate languages before analysis
    for lang in languages {
        validate_language(lang)?;
    }

    let severity_filter = severity.map(|s| s.to_severity());
    let confidence_filter = confidence.map(|c| c.to_confidence());
    let checks_filter = if checks.is_empty() {
        None
    } else {
        Some(checks)
    };

    // Normalize language aliases (e.g., "ts" → "typescript")
    let normalized = normalize_languages(languages);

    tracing::info!("Running diagnostics on {}", canonical.display());

    let (diagnostics, has_findings) = flowspec::diagnose(
        &canonical,
        &config,
        &normalized,
        severity_filter,
        confidence_filter,
        checks_filter,
    )?;

    let output = format_with(format, |f| f.format_diagnostics(&diagnostics))?;

    write_output(&output, output_path)?;

    if has_findings {
        Ok(ExitCode::from(2))
    } else {
        Ok(ExitCode::from(0))
    }
}

/// Run trace command — trace a single symbol's flow through the codebase.
///
/// Runs full analysis, finds the requested symbol, then outputs only the
/// flow data relevant to that symbol. Supports `--depth` for path truncation
/// and `--direction` for trace direction (forward only for now).
#[allow(clippy::too_many_arguments)]
fn run_trace(
    path: &PathBuf,
    symbol: &str,
    languages: &[String],
    depth: usize,
    direction: &TraceDirection,
    format: &Format,
    output_path: Option<&std::path::Path>,
    config_path: Option<&std::path::Path>,
) -> Result<ExitCode, FlowspecError> {
    // Guard unsupported formats
    if !matches!(format, Format::Yaml | Format::Json | Format::Sarif) {
        return Err(FlowspecError::FormatNotImplemented {
            format: format_name(format).to_string(),
        });
    }

    // Guard unsupported directions
    match direction {
        TraceDirection::Forward => {}
        TraceDirection::Backward => {
            return Err(FlowspecError::SymbolNotFound(
                "Backward flow tracing is not yet implemented. Use --direction forward."
                    .to_string(),
            ));
        }
        TraceDirection::Both => {
            return Err(FlowspecError::SymbolNotFound(
                "Bidirectional flow tracing is not yet implemented. Use --direction forward."
                    .to_string(),
            ));
        }
    }

    if path.as_os_str().is_empty() {
        return Err(FlowspecError::EmptyPath);
    }

    let canonical = resolve_path(path)?;
    let config = Config::load(&canonical, config_path)?;

    // Validate and normalize languages
    for lang in languages {
        validate_language(lang)?;
    }
    let normalized_languages = normalize_languages(languages);

    tracing::info!("Tracing symbol '{}' in {}", symbol, canonical.display());

    // Run analysis to get the manifest (includes flows from trace_all_flows)
    let result = flowspec::analyze(&canonical, &config, &normalized_languages)?;

    // Symbol matching: exact qualified name → name part → substring
    let matched_entity = find_matching_symbol(symbol, &result.manifest.entities)?;

    // Filter flows to only those relevant to the matched symbol
    let mut flow_entries: Vec<flowspec::FlowEntry> = result
        .manifest
        .flows
        .into_iter()
        .filter(|flow| {
            flow.entry == matched_entity
                || flow.exit == matched_entity
                || flow.steps.iter().any(|s| s.entity == matched_entity)
        })
        .collect();

    // Apply depth truncation: truncate steps beyond --depth
    for flow in &mut flow_entries {
        if flow.steps.len() > depth {
            flow.steps.truncate(depth);
        }
    }

    // Serialize focused output directly (bypasses OutputFormatter — no manifest wrapper)
    let output = match format {
        Format::Yaml => {
            serde_yaml::to_string(&flow_entries).map_err(|e| FlowspecError::Manifest {
                reason: format!("YAML serialization failed: {}", e),
            })?
        }
        Format::Json => {
            serde_json::to_string_pretty(&flow_entries).map_err(|e| FlowspecError::Manifest {
                reason: format!("JSON serialization failed: {}", e),
            })?
        }
        Format::Sarif => {
            // Wrap flow entries in a minimal SARIF v2.1.0 envelope
            format_trace_sarif(&flow_entries)?
        }
        Format::Summary => {
            return Err(FlowspecError::FormatNotImplemented {
                format: "summary".to_string(),
            })
        }
    };

    write_output(&output, output_path)?;

    Ok(ExitCode::from(0))
}

/// Find a matching symbol in the entity list using cascading match strategy.
///
/// Priority: exact qualified name → exact name part → substring match.
/// Returns an error if no match or multiple ambiguous matches are found.
fn find_matching_symbol(
    symbol: &str,
    entities: &[flowspec::EntityEntry],
) -> Result<String, FlowspecError> {
    // 1. Exact match on qualified name (e.g., "main.py::process")
    let exact_matches: Vec<&flowspec::EntityEntry> =
        entities.iter().filter(|e| e.id == symbol).collect();
    if exact_matches.len() == 1 {
        return Ok(exact_matches[0].id.clone());
    }

    // 2. Exact match on the name part (after last ::)
    let name_matches: Vec<&flowspec::EntityEntry> = entities
        .iter()
        .filter(|e| {
            e.id.rsplit("::")
                .next()
                .map(|name| name == symbol)
                .unwrap_or(false)
        })
        .collect();
    if name_matches.len() == 1 {
        return Ok(name_matches[0].id.clone());
    }
    if name_matches.len() > 1 {
        let options: Vec<&str> = name_matches.iter().map(|e| e.id.as_str()).collect();
        return Err(FlowspecError::SymbolNotFound(format!(
            "Symbol '{}' matches multiple entities: {}. Use a qualified name to disambiguate.",
            symbol,
            options.join(", ")
        )));
    }

    // 3. Substring match
    let substring_matches: Vec<&flowspec::EntityEntry> =
        entities.iter().filter(|e| e.id.contains(symbol)).collect();
    if substring_matches.len() == 1 {
        return Ok(substring_matches[0].id.clone());
    }
    if substring_matches.len() > 1 {
        let options: Vec<&str> = substring_matches.iter().map(|e| e.id.as_str()).collect();
        return Err(FlowspecError::SymbolNotFound(format!(
            "Symbol '{}' matches multiple entities: {}. Use a qualified name to disambiguate.",
            symbol,
            options.join(", ")
        )));
    }

    // No match
    Err(FlowspecError::SymbolNotFound(format!(
        "Symbol '{}' not found. Run `flowspec analyze` to see available entities.",
        symbol
    )))
}

/// Format flow entries as a minimal SARIF v2.1.0 envelope.
fn format_trace_sarif(flow_entries: &[flowspec::FlowEntry]) -> Result<String, FlowspecError> {
    let results: Vec<serde_json::Value> = flow_entries
        .iter()
        .map(|flow| {
            serde_json::json!({
                "ruleId": "flow-trace",
                "message": { "text": flow.description },
                "properties": {
                    "entry": flow.entry,
                    "exit": flow.exit,
                    "steps": flow.steps.iter().map(|s| {
                        serde_json::json!({
                            "entity": s.entity,
                            "action": s.action,
                        })
                    }).collect::<Vec<_>>(),
                }
            })
        })
        .collect();

    let sarif = serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/main/sarif-2.1/schema/sarif-schema-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "flowspec",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://github.com/anthropics/flowspec"
                }
            },
            "results": results,
        }]
    });

    serde_json::to_string_pretty(&sarif).map_err(|e| FlowspecError::Manifest {
        reason: format!("SARIF serialization failed: {}", e),
    })
}

/// Resolve a path, checking existence.
fn resolve_path(path: &PathBuf) -> Result<PathBuf, FlowspecError> {
    if path.as_os_str().is_empty() {
        return Err(FlowspecError::EmptyPath);
    }

    let canonical = if path.is_relative() {
        std::env::current_dir()
            .map_err(|e| FlowspecError::Io {
                path: path.clone(),
                source: e,
            })?
            .join(path)
    } else {
        path.clone()
    };

    if !canonical.exists() {
        return Err(FlowspecError::TargetNotFound { path: path.clone() });
    }

    Ok(canonical)
}

/// Write output to stdout or a file.
fn write_output(content: &str, output_path: Option<&std::path::Path>) -> Result<(), FlowspecError> {
    if let Some(path) = output_path {
        std::fs::write(path, content).map_err(|e| FlowspecError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
    } else {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        handle
            .write_all(content.as_bytes())
            .map_err(|e| FlowspecError::Io {
                path: PathBuf::from("<stdout>"),
                source: e,
            })?;
    }
    Ok(())
}

/// Dispatch formatting to the correct formatter based on the selected format.
fn format_with<F>(format: &Format, f: F) -> Result<String, FlowspecError>
where
    F: FnOnce(&dyn OutputFormatter) -> Result<String, flowspec::ManifestError>,
{
    let result = match format {
        Format::Yaml => f(&YamlFormatter::new()),
        Format::Json => f(&JsonFormatter::new()),
        Format::Sarif => f(&SarifFormatter::new()),
        Format::Summary => {
            return Err(FlowspecError::FormatNotImplemented {
                format: "summary".to_string(),
            })
        }
    };
    result.map_err(FlowspecError::from)
}

/// Get the display name for a format.
fn format_name(format: &Format) -> &'static str {
    match format {
        Format::Yaml => "yaml",
        Format::Json => "json",
        Format::Sarif => "sarif",
        Format::Summary => "summary",
    }
}

/// Normalize a language alias to its canonical name.
///
/// Accepts common abbreviations: "ts" → "typescript", "js" → "javascript",
/// "py" → "python". Canonical names pass through unchanged.
fn normalize_language(lang: &str) -> String {
    match lang {
        "ts" => "typescript".to_string(),
        "js" => "javascript".to_string(),
        "py" => "python".to_string(),
        other => other.to_string(),
    }
}

/// Validate a language name against v1 supported languages.
///
/// Accepts both canonical names and common abbreviations (e.g., "ts" for "typescript").
fn validate_language(lang: &str) -> Result<(), FlowspecError> {
    let normalized = normalize_language(lang);
    match normalized.as_str() {
        "python" | "javascript" | "typescript" | "rust" => Ok(()),
        _ => Err(FlowspecError::UnsupportedLanguage {
            language: lang.to_string(),
        }),
    }
}

/// Normalize a list of language arguments, expanding aliases.
fn normalize_languages(languages: &[String]) -> Vec<String> {
    languages.iter().map(|l| normalize_language(l)).collect()
}
