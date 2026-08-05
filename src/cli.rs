use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "envorigin",
    version,
    about = "Explain where environment variables come from",
    long_about = "EnvOrigin builds a provenance chain for environment variables in Docker Compose projects and GitHub Actions workflows: across shell variables, interpolation files, env_file layers, and environment overrides. Values are redacted unless --show-values is passed."
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
    /// Audit a project for env health issues (sensitive values, dead code).
    Audit(AuditArgs),
    /// Compare environment drift across dotenv files.
    Diff(DiffArgs),
    /// Analyze a GitHub Actions workflow's environment variables.
    Actions(ActionsArgs),
}

#[derive(Debug, Args)]
pub struct DiffArgs {
    /// Dotenv files to compare (two or more).
    #[arg(required = true, num_args = 2..)]
    pub files: Vec<PathBuf>,
    /// Reveal values of sensitive-looking variables too.
    #[arg(long)]
    pub show_values: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct AuditArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Fail (exit 1) when issues of this severity or higher are found.
    #[arg(long, value_enum, default_value_t = FailLevel::Error)]
    pub fail_on: FailLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FailLevel {
    /// Never fail.
    None,
    /// Fail on info issues or higher.
    Info,
    /// Fail on warning issues or higher.
    Warning,
    /// Fail on error issues only.
    Error,
}

impl FailLevel {
    pub fn triggers(self, severity: crate::model::Severity) -> bool {
        use crate::model::Severity;
        use FailLevel::*;
        match (self, severity) {
            (None, _) => false,
            (Error, Severity::Error) => true,
            (Warning, Severity::Warning | Severity::Error) => true,
            (Info, _) => true,
            (_, _) => false,
        }
    }
}

#[derive(Debug, Args)]
pub struct ActionsArgs {
    #[command(subcommand)]
    pub command: ActionsCommand,
}

#[derive(Debug, Subcommand)]
pub enum ActionsCommand {
    /// List the environment variables visible in each job and step.
    Scan(ActionsScanArgs),
    /// Explain the winning and shadowed sources for one variable.
    Explain(ActionsExplainArgs),
    /// Audit a workflow for env health issues.
    Audit(ActionsAuditArgs),
}

#[derive(Debug, Args)]
pub struct ActionsAuditArgs {
    #[command(flatten)]
    pub common: ActionsScanArgs,
    /// Fail (exit 1) when issues of this severity or higher are found.
    #[arg(long, value_enum, default_value_t = FailLevel::Error)]
    pub fail_on: FailLevel,
}

#[derive(Debug, Args)]
pub struct ActionsScanArgs {
    /// Workflow file to analyze.
    #[arg(long = "file", short = 'f', default_value = ".github/workflows/ci.yml")]
    pub workflow_file: PathBuf,
    /// Override the repository root (env files resolve relative to it).
    #[arg(long)]
    pub project_directory: Option<PathBuf>,
    /// Reveal values. By default values are replaced by a short SHA-256 fingerprint.
    #[arg(long)]
    pub show_values: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct ActionsExplainArgs {
    /// Environment variable name to explain.
    pub variable: String,
    /// Job to inspect. Optional when the workflow has a single job.
    #[arg(long, short = 'j')]
    pub job: Option<String>,
    /// Step name, id, or index to inspect. Omit to explain the job-level variable.
    #[arg(long, short = 's')]
    pub step: Option<String>,
    #[command(flatten)]
    pub common: ActionsScanArgs,
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
