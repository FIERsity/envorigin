use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

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
    /// Render a mermaid provenance graph of the environment.
    Graph(GraphArgs),
    /// Analyze a GitLab CI configuration's variables.
    Gitlab(GitlabArgs),
    /// Analyze a CircleCI configuration's environment variables.
    Circleci(CircleciArgs),
    /// Analyze a GitHub Actions workflow's environment variables.
    Actions(ActionsArgs),
    /// Start the LSP server (for editor integration).
    Lsp,
    /// Generate an envorigin.toml rules template in the current directory.
    Init,
    /// Audit standalone dotenv files without a Compose context.
    Dotenv(DotenvArgs),
    /// Generate a shell completion script.
    Completions(CompletionsArgs),
}

#[derive(Debug, Args)]
pub struct CircleciArgs {
    #[command(subcommand)]
    pub command: CircleciCommand,
}

#[derive(Debug, Subcommand)]
pub enum CircleciCommand {
    /// List the environment variables defined per job.
    Scan(CircleciScanArgs),
    /// Explain the winning and shadowed sources for one variable.
    Explain(CircleciExplainArgs),
    /// Audit the configuration for env health issues.
    Audit(CircleciAuditArgs),
    /// Render a mermaid provenance graph of the variables.
    Graph(CircleciScanArgs),
}

#[derive(Debug, Args)]
pub struct CircleciAuditArgs {
    #[command(flatten)]
    pub common: CircleciScanArgs,
    /// Fail (exit 1) when issues of this severity or higher are found.
    #[arg(long, value_enum, default_value_t = FailLevel::Error)]
    pub fail_on: FailLevel,
    /// Team convention rules file (default: ./envorigin.toml if present).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Ignore issues with this code (repeatable). Use for CI migrations:
    /// exempt legacy problems, then remove ignores as they are fixed.
    #[arg(long)]
    pub ignore: Vec<String>,
}

#[derive(Debug, Args)]
pub struct CircleciScanArgs {
    /// CircleCI config file to analyze.
    #[arg(long = "file", short = 'f', default_value = ".circleci/config.yml")]
    pub file: PathBuf,
    /// Reveal values. By default values are replaced by a short SHA-256 fingerprint.
    #[arg(long)]
    pub show_values: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct CircleciExplainArgs {
    /// Environment variable name to explain.
    pub variable: String,
    /// Job to inspect.
    #[arg(long, short = 'j')]
    pub job: String,
    #[command(flatten)]
    pub common: CircleciScanArgs,
}

#[derive(Debug, Args)]
pub struct GitlabArgs {
    #[command(subcommand)]
    pub command: GitlabCommand,
}

#[derive(Debug, Subcommand)]
pub enum GitlabCommand {
    /// List the variables defined globally and per job.
    Scan(GitlabScanArgs),
    /// Explain the winning and shadowed sources for one variable.
    Explain(GitlabExplainArgs),
    /// Audit the configuration for env health issues.
    Audit(GitlabAuditArgs),
    /// Render a mermaid provenance graph of the variables.
    Graph(GitlabScanArgs),
}

#[derive(Debug, Args)]
pub struct GitlabAuditArgs {
    #[command(flatten)]
    pub common: GitlabScanArgs,
    /// Fail (exit 1) when issues of this severity or higher are found.
    #[arg(long, value_enum, default_value_t = FailLevel::Error)]
    pub fail_on: FailLevel,
    /// Team convention rules file (default: ./envorigin.toml if present).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Ignore issues with this code (repeatable). Use for CI migrations:
    /// exempt legacy problems, then remove ignores as they are fixed.
    #[arg(long)]
    pub ignore: Vec<String>,
}

#[derive(Debug, Args)]
pub struct GitlabScanArgs {
    /// GitLab CI file to analyze.
    #[arg(long = "file", short = 'f', default_value = ".gitlab-ci.yml")]
    pub file: PathBuf,
    /// Reveal values. By default values are replaced by a short SHA-256 fingerprint.
    #[arg(long)]
    pub show_values: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct GitlabExplainArgs {
    /// Environment variable name to explain.
    pub variable: String,
    /// Job to inspect. Omit to explain the file-global variable.
    #[arg(long, short = 'j')]
    pub job: Option<String>,
    #[command(flatten)]
    pub common: GitlabScanArgs,
}

#[derive(Debug, Args)]
pub struct GraphArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Debug, Args)]
pub struct DiffArgs {
    /// Dotenv files to compare (two or more).
    #[arg(required = true, num_args = 2.., conflicts_with_all = ["project_a", "project_b"])]
    pub files: Vec<PathBuf>,
    /// Compare the final environments of two Compose projects instead.
    #[arg(long, conflicts_with = "files")]
    pub project_a: Option<PathBuf>,
    /// Second Compose project directory.
    #[arg(long, requires = "project_a")]
    pub project_b: Option<PathBuf>,
    /// Reveal values of sensitive-looking variables too.
    #[arg(long)]
    pub show_values: bool,
    /// Exit 1 when drift is found (same key with different values). Useful in CI.
    #[arg(long)]
    pub fail_on_drift: bool,
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
    /// Team convention rules file (default: ./envorigin.toml if present).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Ignore issues with this code (repeatable). Use for CI migrations:
    /// exempt legacy problems, then remove ignores as they are fixed.
    #[arg(long)]
    pub ignore: Vec<String>,
}

#[derive(Debug, Args)]
pub struct DotenvArgs {
    /// Dotenv files to audit.
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
    /// Fail (exit 1) when issues of this severity or higher are found.
    #[arg(long, value_enum, default_value_t = FailLevel::Error)]
    pub fail_on: FailLevel,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,
    /// Team convention rules file (default: ./envorigin.toml if present).
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// Shell to generate completions for.
    pub shell: Shell,
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
    /// Render a mermaid provenance graph of the workflow env.
    Graph(ActionsScanArgs),
}

#[derive(Debug, Args)]
pub struct ActionsAuditArgs {
    #[command(flatten)]
    pub common: ActionsScanArgs,
    /// Fail (exit 1) when issues of this severity or higher are found.
    #[arg(long, value_enum, default_value_t = FailLevel::Error)]
    pub fail_on: FailLevel,
    /// Team convention rules file (default: ./envorigin.toml if present).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Ignore issues with this code (repeatable). Use for CI migrations:
    /// exempt legacy problems, then remove ignores as they are fixed.
    #[arg(long)]
    pub ignore: Vec<String>,
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
    /// Print the full layer-by-layer resolution trace for the variable.
    #[arg(long)]
    pub debug: bool,
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
    /// Print the full resolution trace for the variable.
    #[arg(long)]
    pub debug: bool,
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
    /// GitHub Actions workflow commands (::error file=...,line=...::...).
    Github,
}
