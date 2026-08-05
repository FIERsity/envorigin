use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "envorigin",
    version,
    about = "Explain where Docker Compose environment variables come from",
    long_about = "EnvOrigin builds a provenance chain across shell variables, Compose interpolation files, service env_file entries, and service environment overrides. Values are redacted unless --show-values is passed."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List the variables configured for each Compose service.
    Scan(ScanArgs),
    /// Explain the winning and shadowed sources for one variable.
    Explain(ExplainArgs),
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Limit output to a single service.
    #[arg(long)]
    pub service: Option<String>,
}

#[derive(Debug, Args)]
pub struct ExplainArgs {
    /// Environment variable name to explain.
    pub variable: String,
    /// Compose service. Optional when the file contains only one service.
    #[arg(long, short = 's')]
    pub service: Option<String>,
    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Debug, Args)]
pub struct CommonArgs {
    /// Compose file to analyze.
    #[arg(long = "file", short = 'f', default_value = "compose.yaml")]
    pub compose_file: PathBuf,
    /// Compose interpolation env file. Repeat for multiple files; later files win.
    #[arg(long = "env-file")]
    pub env_files: Vec<PathBuf>,
    /// Override the Compose project directory.
    #[arg(long)]
    pub project_directory: Option<PathBuf>,
    /// Overlay the process environment from a dotenv file for reproducible diagnosis.
    #[arg(long)]
    pub host_env_file: Option<PathBuf>,
    /// Skip comparison with `docker compose config`.
    #[arg(long)]
    pub no_docker_check: bool,
    /// Reveal values. By default values are replaced by a short SHA-256 fingerprint.
    #[arg(long)]
    pub show_values: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}
