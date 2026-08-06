use std::fmt::Write as _;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use envorigin::actions::{
    actions_human, actions_json, actions_variable_human, actions_variable_json,
};
use envorigin::audit::{
    audit_circleci, audit_gitlab, audit_human, audit_project, audit_workflow, AuditIssue,
};
use envorigin::circleci::{
    analyze_circleci, circleci_human, circleci_json, circleci_variable_human,
    circleci_variable_json,
};
use envorigin::cli::{
    ActionsCommand, CircleciCommand, Cli, Command, CommonArgs, FailLevel, GitlabCommand,
    OutputFormat,
};
use envorigin::detect::Target;
use envorigin::diff::{
    diff_files, diff_human, diff_json, diff_projects, project_diff_human, project_diff_json,
};
use envorigin::gitlab::{
    analyze_gitlab, gitlab_human, gitlab_json, gitlab_variable_human, gitlab_variable_json,
};
use envorigin::graph::{actions_graph, circleci_graph, gitlab_graph, project_graph};
use envorigin::model::AnalysisError;
use envorigin::output::{
    explanation_human, explanation_json, project_human, project_json, service_human,
};
use envorigin::rules::{default_config_path, Rules};
use envorigin::{analyze, AnalyzeOptions};

/// A writer that swallows EPIPE: completion scripts piped into `head` or
/// `grep` must not panic when the consumer closes the pipe early.
struct IgnoreBrokenPipe(std::io::Stdout);

impl Write for IgnoreBrokenPipe {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.0.write(buf) {
            Err(error) if error.kind() == ErrorKind::BrokenPipe => Ok(buf.len()),
            result => result,
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// Write an envorigin.toml rules template; never overwrites an existing
/// file.
fn init_rules() -> ExitCode {
    const TEMPLATE: &str = r#"# EnvOrigin team conventions.
# Remove or fill in what your project needs; all sections are optional.

# Variables that must resolve to a value somewhere in the project.
# required = ["DATABASE_URL", "LOG_LEVEL"]

# Every user-defined variable must start with this prefix.
# prefix = "APP_"

# Variables that must not be defined at all.
# forbidden = ["CI"]

# Value format validation (regex).
# [patterns]
# DATABASE_URL = "^postgres(ql)?://"

# Enum whitelists.
# [allowed]
# DB_ENGINE = ["postgres", "mysql"]

# Length caps.
# [max_length]
# API_KEY = 64
"#;
    let path = std::path::Path::new("envorigin.toml");
    if path.exists() {
        eprintln!("envorigin.toml already exists; leaving it untouched");
        return ExitCode::SUCCESS;
    }
    match std::fs::write(path, TEMPLATE) {
        Ok(()) => {
            println!("wrote envorigin.toml (rules template)");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: failed to write envorigin.toml: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Render issues as GitHub Actions workflow commands so problems appear
/// annotated on the offending file lines in pull requests.
/// Resolution trace for `explain --debug`: every definition of the
/// variable in the interpolation context, lowest precedence first.
fn debug_trace(report: &envorigin::model::ProjectReport, service: &str, variable: &str) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "resolution trace for {variable} (service {service}):"
    );
    let mut context =
        envorigin::interpolation::InterpolationContext::<envorigin::model::SourceRef>::new();
    for (key, value) in std::env::vars() {
        context.define(
            key,
            Some(value),
            envorigin::model::SourceRef::new(
                envorigin::model::SourceKind::HostEnvironment,
                None,
                None,
                "process environment",
            ),
            10_000,
        );
    }
    for (index, path) in report.interpolation_files.iter().enumerate() {
        if let Ok(parsed) = envorigin::dotenv::parse_dotenv_file(path) {
            for entry in parsed.entries {
                context.define(
                    entry.key.clone(),
                    entry.value,
                    envorigin::model::SourceRef::new(
                        envorigin::model::SourceKind::DefaultEnvFile,
                        Some(path.clone()),
                        Some(entry.line),
                        "interpolation file",
                    ),
                    100 + index as i32,
                );
            }
        }
    }
    let entries = context.debug(variable);
    for entry in &entries {
        let value = entry
            .value
            .as_deref()
            .map(|value| format!("{value:?}"))
            .unwrap_or_else(|| "—".to_string());
        let _ = writeln!(
            output,
            "  precedence {:>5} order {:>3}  {:?}  {value}",
            entry.precedence, entry.order, entry.source.kind
        );
    }
    let _ = writeln!(
        output,
        "  {} definition(s) in the interpolation context",
        entries.len()
    );
    output
}

fn github_annotations(issues: &[&AuditIssue]) -> String {
    issues
        .iter()
        .map(|issue| {
            let level = match issue.severity {
                envorigin::model::Severity::Error => "error",
                envorigin::model::Severity::Warning => "warning",
                envorigin::model::Severity::Info => "notice",
            };
            let location = match (&issue.path, issue.line) {
                (Some(path), Some(line)) => format!("file={},line={}", path.display(), line),
                (Some(path), None) => format!("file={}", path.display()),
                (None, _) => String::new(),
            };
            let message = issue.message.replace('%', "%25").replace('\n', "%0A");
            if location.is_empty() {
                format!("::{level}::{message}")
            } else {
                format!("::{level} {location}::{message}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct RunOutcome {
    output: String,
    exit_code: ExitCode,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match &cli.command {
        Command::Lsp => {
            envorigin::lsp::run_lsp();
            return ExitCode::SUCCESS;
        }
        Command::Completions(args) => {
            let mut command = Cli::command();
            clap_complete::generate(
                args.shell,
                &mut command,
                "envorigin",
                &mut IgnoreBrokenPipe(std::io::stdout()),
            );
            return ExitCode::SUCCESS;
        }
        Command::Init => {
            return init_rules();
        }
        Command::Dotenv(args) => {
            return match run_dotenv_audit(&args.files, args.format, args.fail_on, &[], &args.config)
            {
                Ok(outcome) => {
                    println!("{}", outcome.output);
                    outcome.exit_code
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    ExitCode::FAILURE
                }
            };
        }
        _ => {}
    }
    match run(cli) {
        Ok(outcome) => {
            println!("{}", outcome.output);
            outcome.exit_code
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<RunOutcome, AnalysisError> {
    let outcome_with_code = |output: String, exit_code: ExitCode| RunOutcome { output, exit_code };
    let outcome = |output: String| outcome_with_code(output, ExitCode::SUCCESS);
    match cli.command {
        Command::Scan(args) => match resolve_target(&args.common, args.path.as_deref())? {
            Some(Target::Compose { file }) => {
                run_compose_scan(&file, &args.common, args.service.as_deref())
            }
            Some(Target::Actions { file }) => run_actions_scan(
                &file,
                args.common.project_directory.as_deref(),
                args.common.format,
                args.common.show_values,
            ),
            Some(Target::Gitlab { file }) => {
                run_gitlab_scan(&file, args.common.format, args.common.show_values)
            }
            Some(Target::Circleci { file }) => {
                run_circleci_scan(&file, args.common.format, args.common.show_values)
            }
            Some(Target::Dotenv { .. }) => Err(AnalysisError::AutoDetectedNoCommand {
                kind: "dotenv".to_string(),
                command: "scan".to_string(),
                suggestion: "use `envorigin dotenv audit`".to_string(),
            }),
            None => run_compose_scan(
                &args
                    .common
                    .compose_file
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("compose.yaml")),
                &args.common,
                args.service.as_deref(),
            ),
        },
        Command::Explain(args) => match resolve_target(&args.common, args.path.as_deref())? {
            Some(Target::Compose { file }) => run_compose_explain(
                &file,
                &args.common,
                &args.variable,
                args.service.as_deref(),
                args.debug,
            ),
            Some(Target::Actions { file }) => run_actions_explain(
                &file,
                &args.variable,
                None,
                None,
                args.debug,
                args.common.format,
                args.common.show_values,
            ),
            Some(Target::Gitlab { file }) => run_gitlab_explain(
                &file,
                &args.variable,
                None,
                args.common.format,
                args.common.show_values,
            ),
            Some(Target::Circleci { file }) => run_circleci_explain_auto(
                &file,
                &args.variable,
                args.common.format,
                args.common.show_values,
            ),
            Some(Target::Dotenv { .. }) => Err(AnalysisError::AutoDetectedNoCommand {
                kind: "dotenv".to_string(),
                command: "explain".to_string(),
                suggestion: "use `envorigin dotenv audit`".to_string(),
            }),
            None => run_compose_explain(
                &args
                    .common
                    .compose_file
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("compose.yaml")),
                &args.common,
                &args.variable,
                args.service.as_deref(),
                args.debug,
            ),
        },
        Command::Audit(args) => match resolve_target(&args.common, args.path.as_deref())? {
            Some(Target::Compose { file }) => run_compose_audit(
                &file,
                &args.common,
                args.fail_on,
                &args.ignore,
                &args.config,
            ),
            Some(Target::Actions { file }) => run_actions_audit(
                &file,
                args.common.format,
                args.fail_on,
                &args.ignore,
                &args.config,
            ),
            Some(Target::Gitlab { file }) => run_gitlab_audit(
                &file,
                args.common.format,
                args.fail_on,
                &args.ignore,
                &args.config,
            ),
            Some(Target::Circleci { file }) => run_circleci_audit(
                &file,
                args.common.format,
                args.fail_on,
                &args.ignore,
                &args.config,
            ),
            Some(Target::Dotenv { file }) => run_dotenv_audit(
                std::slice::from_ref(&file),
                args.common.format,
                args.fail_on,
                &args.ignore,
                &args.config,
            ),
            None => run_compose_audit(
                &args
                    .common
                    .compose_file
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("compose.yaml")),
                &args.common,
                args.fail_on,
                &args.ignore,
                &args.config,
            ),
        },
        Command::Diff(args) => {
            if let (Some(a), Some(b)) = (&args.project_a, &args.project_b) {
                let report_a = analyze(&AnalyzeOptions {
                    compose_file: a.join("compose.yaml"),
                    docker_check: false,
                    ..AnalyzeOptions::default()
                })?;
                let report_b = analyze(&AnalyzeOptions {
                    compose_file: b.join("compose.yaml"),
                    docker_check: false,
                    ..AnalyzeOptions::default()
                })?;
                let entries = diff_projects(&report_a, &report_b);
                let drifted = entries.iter().any(|entry| {
                    entry.values[0].is_some()
                        && entry.values[1].is_some()
                        && entry.values[0] != entry.values[1]
                });
                let output = match args.format {
                    OutputFormat::Json => project_diff_json(&entries, args.show_values),
                    OutputFormat::Human | OutputFormat::Github => project_diff_human(
                        &a.display().to_string(),
                        &b.display().to_string(),
                        &entries,
                        args.show_values,
                    ),
                };
                return Ok(outcome_with_code(
                    output,
                    if args.fail_on_drift && drifted {
                        ExitCode::FAILURE
                    } else {
                        ExitCode::SUCCESS
                    },
                ));
            }
            let report = diff_files(&args.files)?;
            let drifted = !report.drifts().is_empty();
            let output = match args.format {
                OutputFormat::Human => diff_human(&report, args.show_values),
                OutputFormat::Github => diff_human(&report, args.show_values),
                OutputFormat::Json => diff_json(&report, args.show_values),
            };
            Ok(outcome_with_code(
                output,
                if args.fail_on_drift && drifted {
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                },
            ))
        }
        Command::Graph(args) => match resolve_target(&args.common, args.path.as_deref())? {
            Some(Target::Compose { file }) => {
                let report = analyze(&compose_options(&file, &args.common))?;
                Ok(outcome(project_graph(&report)))
            }
            Some(Target::Actions { file }) => run_actions_graph(&file),
            Some(Target::Gitlab { file }) => run_gitlab_graph(&file),
            Some(Target::Circleci { file }) => run_circleci_graph(&file),
            Some(Target::Dotenv { .. }) => Err(AnalysisError::AutoDetectedNoCommand {
                kind: "dotenv".to_string(),
                command: "graph".to_string(),
                suggestion: "use `envorigin dotenv audit`".to_string(),
            }),
            None => {
                let report = analyze(&compose_options(
                    &args
                        .common
                        .compose_file
                        .clone()
                        .unwrap_or_else(|| PathBuf::from("compose.yaml")),
                    &args.common,
                ))?;
                Ok(outcome(project_graph(&report)))
            }
        },
        Command::Circleci(args) => match args.command {
            CircleciCommand::Scan(scan) => {
                run_circleci_scan(&scan.file, scan.format, scan.show_values)
            }
            CircleciCommand::Explain(explain) => run_circleci_explain(
                &explain.common.file,
                &explain.job,
                &explain.variable,
                explain.common.format,
                explain.common.show_values,
            ),
            CircleciCommand::Graph(graph) => run_circleci_graph(&graph.file),
            CircleciCommand::Audit(audit) => run_circleci_audit(
                &audit.common.file,
                audit.common.format,
                audit.fail_on,
                &audit.ignore,
                &audit.config,
            ),
        },
        Command::Gitlab(args) => match args.command {
            GitlabCommand::Scan(scan) => run_gitlab_scan(&scan.file, scan.format, scan.show_values),
            GitlabCommand::Explain(explain) => run_gitlab_explain(
                &explain.common.file,
                &explain.variable,
                explain.job.as_deref(),
                explain.common.format,
                explain.common.show_values,
            ),
            GitlabCommand::Graph(graph) => run_gitlab_graph(&graph.file),
            GitlabCommand::Audit(audit) => run_gitlab_audit(
                &audit.common.file,
                audit.common.format,
                audit.fail_on,
                &audit.ignore,
                &audit.config,
            ),
        },
        // Handled in main() before run() is called.
        Command::Lsp => Ok(outcome(String::new())),
        Command::Init => Ok(outcome(String::new())),
        Command::Dotenv(_) => Ok(outcome(String::new())),
        Command::Completions(_) => Ok(outcome(String::new())),
        Command::Actions(args) => match args.command {
            ActionsCommand::Scan(scan) => run_actions_scan(
                &scan.workflow_file,
                scan.project_directory.as_deref(),
                scan.format,
                scan.show_values,
            ),
            ActionsCommand::Explain(explain) => run_actions_explain(
                &explain.common.workflow_file,
                &explain.variable,
                explain.job.as_deref(),
                explain.step.as_deref(),
                explain.debug,
                explain.common.format,
                explain.common.show_values,
            ),
            ActionsCommand::Audit(audit) => run_actions_audit(
                &audit.common.workflow_file,
                audit.common.format,
                audit.fail_on,
                &audit.ignore,
                &audit.config,
            ),
            ActionsCommand::Graph(graph) => run_actions_graph(&graph.workflow_file),
        },
    }
}

fn exit_code_for_audit(
    issues: &[AuditIssue],
    fail_on: FailLevel,
    format: OutputFormat,
    ignores: &[String],
    human: impl FnOnce(&[&AuditIssue]) -> String,
) -> Result<RunOutcome, AnalysisError> {
    let issues: Vec<&AuditIssue> = issues
        .iter()
        .filter(|issue| !ignores.iter().any(|code| code == &issue.code))
        .collect();
    let failed = issues.iter().any(|issue| fail_on.triggers(issue.severity));
    let output = match format {
        OutputFormat::Human => human(&issues),
        OutputFormat::Json => {
            serde_json::to_string_pretty(&issues).expect("AuditIssue is serializable")
        }
        OutputFormat::Github => github_annotations(&issues),
    };
    Ok(RunOutcome {
        output,
        exit_code: if failed {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        },
    })
}

fn load_rules(config: &Option<std::path::PathBuf>) -> Result<Option<Rules>, AnalysisError> {
    let path = config.clone().or_else(default_config_path);
    let Some(path) = path else {
        return Ok(None);
    };
    Rules::from_file(&path).map_err(|message| AnalysisError::InvalidRules { path, message })
}

fn select_job<'a>(
    report: &'a envorigin::actions::ActionsReport,
    requested: Option<&'a str>,
) -> Result<&'a str, AnalysisError> {
    if let Some(job) = requested {
        if report.jobs.iter().any(|candidate| candidate.id == job) {
            return Ok(job);
        }
        return Err(AnalysisError::UnknownJob(job.to_string()));
    }
    if report.jobs.len() == 1 {
        return Ok(&report.jobs[0].id);
    }
    Err(AnalysisError::UnknownJob(
        report
            .jobs
            .iter()
            .map(|job| job.id.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    ))
}

fn select_step(
    report: &envorigin::actions::ActionsReport,
    job: &str,
    spec: &str,
) -> Result<usize, AnalysisError> {
    let job_report = report
        .jobs
        .iter()
        .find(|candidate| candidate.id == job)
        .expect("job was selected");
    if let Ok(index) = spec.parse::<usize>() {
        if let Some(step) = job_report.steps.iter().find(|step| step.index == index) {
            return Ok(step.index);
        }
    }
    let by_name = job_report
        .steps
        .iter()
        .find(|step| step.name.as_deref() == Some(spec) || step.id.as_deref() == Some(spec));
    match by_name {
        Some(step) => Ok(step.index),
        None => Err(AnalysisError::UnknownStep {
            job: job.to_string(),
            step: spec.to_string(),
        }),
    }
}

fn compose_options(file: &Path, common: &CommonArgs) -> AnalyzeOptions {
    AnalyzeOptions {
        compose_file: file.to_path_buf(),
        env_files: common.env_files.clone(),
        project_directory: common.project_directory.clone(),
        host_env_file: common.host_env_file.clone(),
        docker_check: !common.no_docker_check,
    }
}

/// Resolve the analysis target for the top-level commands when the user
/// did not pass `--file`: a positional path wins, then auto-detection in
/// the project directory, then the explicit compose default (`None`).
fn resolve_target(
    common: &CommonArgs,
    path: Option<&Path>,
) -> Result<Option<Target>, AnalysisError> {
    if let Some(path) = path {
        return envorigin::detect::target_from_path(path)
            .map(Some)
            .ok_or_else(|| AnalysisError::NoConfigFound {
                path: path.to_path_buf(),
            });
    }
    if common.compose_file.is_some() {
        return Ok(None);
    }
    let dir = common
        .project_directory
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    match envorigin::detect::detect_in(&dir) {
        Some(target) => Ok(Some(target)),
        None => Err(AnalysisError::NoConfigFound { path: dir }),
    }
}

/// Append a hint when values were hidden, so first-time users learn the
/// flag exists. Machine-readable formats stay untouched.
fn with_redaction_hint(mut output: String, show_values: bool, format: OutputFormat) -> String {
    if !show_values && matches!(format, OutputFormat::Human | OutputFormat::Github) {
        let _ = writeln!(
            output,
            "\nvalues are hidden; pass --show-values to reveal them"
        );
    }
    output
}

fn run_compose_scan(
    file: &Path,
    common: &CommonArgs,
    service: Option<&str>,
) -> Result<RunOutcome, AnalysisError> {
    let report = analyze(&compose_options(file, common))?;
    let output = match service {
        Some(service_name) => {
            let service = report
                .services
                .iter()
                .find(|service| service.name == service_name)
                .ok_or_else(|| AnalysisError::UnknownService(service_name.to_string()))?;
            match common.format {
                OutputFormat::Human => service_human(&report, service, common.show_values),
                OutputFormat::Github => service_human(&report, service, common.show_values),
                OutputFormat::Json => {
                    let mut filtered = report.clone();
                    filtered.services = vec![service.clone()];
                    project_json(&filtered, common.show_values)
                }
            }
        }
        None => match common.format {
            OutputFormat::Human => project_human(&report, common.show_values),
            OutputFormat::Github => project_human(&report, common.show_values),
            OutputFormat::Json => project_json(&report, common.show_values),
        },
    };
    Ok(RunOutcome {
        output: with_redaction_hint(output, common.show_values, common.format),
        exit_code: ExitCode::SUCCESS,
    })
}

fn run_compose_explain(
    file: &Path,
    common: &CommonArgs,
    variable: &str,
    service: Option<&str>,
    debug: bool,
) -> Result<RunOutcome, AnalysisError> {
    let report = analyze(&compose_options(file, common))?;
    let service = select_service(&report, service)?;
    let explanation = report.explain(service, variable)?;
    let mut output = match common.format {
        OutputFormat::Human => explanation_human(explanation, common.show_values),
        OutputFormat::Github => explanation_human(explanation, common.show_values),
        OutputFormat::Json => explanation_json(explanation, common.show_values),
    };
    if debug {
        output = format!("{}\n{}", debug_trace(&report, service, variable), output);
    }
    Ok(RunOutcome {
        output: with_redaction_hint(output, common.show_values, common.format),
        exit_code: ExitCode::SUCCESS,
    })
}

fn run_compose_audit(
    file: &Path,
    common: &CommonArgs,
    fail_on: FailLevel,
    ignores: &[String],
    config: &Option<PathBuf>,
) -> Result<RunOutcome, AnalysisError> {
    let report = analyze(&compose_options(file, common))?;
    let rules = load_rules(config)?;
    let issues = audit_project(&report, rules.as_ref());
    exit_code_for_audit(&issues, fail_on, common.format, ignores, |filtered| {
        let owned: Vec<AuditIssue> = filtered.iter().map(|issue| (*issue).clone()).collect();
        audit_human(&format!("compose: {}", file.display()), &owned)
    })
}

fn run_actions_scan(
    file: &Path,
    project_directory: Option<&Path>,
    format: OutputFormat,
    show_values: bool,
) -> Result<RunOutcome, AnalysisError> {
    let report = envorigin::actions::analyze_workflow(file, project_directory)?;
    let output = match format {
        OutputFormat::Human => actions_human(&report, show_values),
        OutputFormat::Github => actions_human(&report, show_values),
        OutputFormat::Json => actions_json(&report, show_values),
    };
    Ok(RunOutcome {
        output: with_redaction_hint(output, show_values, format),
        exit_code: ExitCode::SUCCESS,
    })
}

fn run_actions_explain(
    file: &Path,
    variable: &str,
    job: Option<&str>,
    step: Option<&str>,
    debug: bool,
    format: OutputFormat,
    show_values: bool,
) -> Result<RunOutcome, AnalysisError> {
    let report = envorigin::actions::analyze_workflow(file, None)?;
    let job = select_job(&report, job)?;
    let step_index = match step {
        None => None,
        Some(spec) => Some(select_step(&report, job, spec)?),
    };
    let explanation = report.explain(job, step_index, variable)?;
    let mut output = match format {
        OutputFormat::Human => actions_variable_human(&report, explanation, show_values),
        OutputFormat::Github => actions_variable_human(&report, explanation, show_values),
        OutputFormat::Json => actions_variable_json(&report, explanation, show_values),
    };
    if debug && !matches!(format, OutputFormat::Json) {
        output = format!(
            "{}\n{}",
            envorigin::actions::actions_debug_trace(explanation),
            output
        );
    }
    Ok(RunOutcome {
        output: with_redaction_hint(output, show_values, format),
        exit_code: ExitCode::SUCCESS,
    })
}

fn run_actions_audit(
    file: &Path,
    format: OutputFormat,
    fail_on: FailLevel,
    ignores: &[String],
    config: &Option<PathBuf>,
) -> Result<RunOutcome, AnalysisError> {
    let report = envorigin::actions::analyze_workflow(file, None)?;
    let rules = load_rules(config)?;
    let issues = audit_workflow(&report, rules.as_ref());
    exit_code_for_audit(&issues, fail_on, format, ignores, |filtered| {
        let owned: Vec<AuditIssue> = filtered.iter().map(|issue| (*issue).clone()).collect();
        audit_human(
            &format!("workflow: {}", report.workflow_file.display()),
            &owned,
        )
    })
}

fn run_actions_graph(file: &Path) -> Result<RunOutcome, AnalysisError> {
    let report = envorigin::actions::analyze_workflow(file, None)?;
    Ok(RunOutcome {
        output: actions_graph(&report),
        exit_code: ExitCode::SUCCESS,
    })
}

fn run_gitlab_scan(
    file: &Path,
    format: OutputFormat,
    show_values: bool,
) -> Result<RunOutcome, AnalysisError> {
    let report = analyze_gitlab(file)?;
    let output = match format {
        OutputFormat::Human => gitlab_human(&report, show_values),
        OutputFormat::Github => gitlab_human(&report, show_values),
        OutputFormat::Json => gitlab_json(&report, show_values),
    };
    Ok(RunOutcome {
        output: with_redaction_hint(output, show_values, format),
        exit_code: ExitCode::SUCCESS,
    })
}

fn run_gitlab_explain(
    file: &Path,
    variable: &str,
    job: Option<&str>,
    format: OutputFormat,
    show_values: bool,
) -> Result<RunOutcome, AnalysisError> {
    let report = analyze_gitlab(file)?;
    let explanation = report.explain(job, variable)?;
    let output = match format {
        OutputFormat::Human => gitlab_variable_human(&report, explanation, show_values),
        OutputFormat::Github => gitlab_variable_human(&report, explanation, show_values),
        OutputFormat::Json => gitlab_variable_json(&report, explanation, show_values),
    };
    Ok(RunOutcome {
        output: with_redaction_hint(output, show_values, format),
        exit_code: ExitCode::SUCCESS,
    })
}

fn run_gitlab_audit(
    file: &Path,
    format: OutputFormat,
    fail_on: FailLevel,
    ignores: &[String],
    config: &Option<PathBuf>,
) -> Result<RunOutcome, AnalysisError> {
    let report = analyze_gitlab(file)?;
    let rules = load_rules(config)?;
    let issues = audit_gitlab(&report, rules.as_ref());
    exit_code_for_audit(&issues, fail_on, format, ignores, |filtered| {
        let owned: Vec<AuditIssue> = filtered.iter().map(|issue| (*issue).clone()).collect();
        audit_human(&format!("file: {}", report.file.display()), &owned)
    })
}

fn run_gitlab_graph(file: &Path) -> Result<RunOutcome, AnalysisError> {
    let report = analyze_gitlab(file)?;
    Ok(RunOutcome {
        output: gitlab_graph(&report),
        exit_code: ExitCode::SUCCESS,
    })
}

fn run_circleci_scan(
    file: &Path,
    format: OutputFormat,
    show_values: bool,
) -> Result<RunOutcome, AnalysisError> {
    let report = analyze_circleci(file)?;
    let output = match format {
        OutputFormat::Human => circleci_human(&report, show_values),
        OutputFormat::Github => circleci_human(&report, show_values),
        OutputFormat::Json => circleci_json(&report, show_values),
    };
    Ok(RunOutcome {
        output: with_redaction_hint(output, show_values, format),
        exit_code: ExitCode::SUCCESS,
    })
}

fn run_circleci_explain(
    file: &Path,
    job: &str,
    variable: &str,
    format: OutputFormat,
    show_values: bool,
) -> Result<RunOutcome, AnalysisError> {
    let report = analyze_circleci(file)?;
    let explanation = report.explain(job, variable)?;
    let output = match format {
        OutputFormat::Human => circleci_variable_human(&report, explanation, show_values),
        OutputFormat::Github => circleci_variable_human(&report, explanation, show_values),
        OutputFormat::Json => circleci_variable_json(&report, explanation, show_values),
    };
    Ok(RunOutcome {
        output: with_redaction_hint(output, show_values, format),
        exit_code: ExitCode::SUCCESS,
    })
}

/// Auto-detected CircleCI explain: pick the first job (the top-level
/// `explain` has no `-j` flag).
fn run_circleci_explain_auto(
    file: &Path,
    variable: &str,
    format: OutputFormat,
    show_values: bool,
) -> Result<RunOutcome, AnalysisError> {
    let report = analyze_circleci(file)?;
    let job = report
        .jobs
        .first()
        .ok_or_else(|| AnalysisError::UnknownJob("the config defines no jobs".to_string()))?;
    let explanation = report.explain(&job.id, variable)?;
    let output = match format {
        OutputFormat::Human => circleci_variable_human(&report, explanation, show_values),
        OutputFormat::Github => circleci_variable_human(&report, explanation, show_values),
        OutputFormat::Json => circleci_variable_json(&report, explanation, show_values),
    };
    Ok(RunOutcome {
        output: with_redaction_hint(output, show_values, format),
        exit_code: ExitCode::SUCCESS,
    })
}

fn run_circleci_audit(
    file: &Path,
    format: OutputFormat,
    fail_on: FailLevel,
    ignores: &[String],
    config: &Option<PathBuf>,
) -> Result<RunOutcome, AnalysisError> {
    let report = analyze_circleci(file)?;
    let rules = load_rules(config)?;
    let issues = audit_circleci(&report, rules.as_ref());
    exit_code_for_audit(&issues, fail_on, format, ignores, |filtered| {
        let owned: Vec<AuditIssue> = filtered.iter().map(|issue| (*issue).clone()).collect();
        audit_human(&format!("file: {}", report.file.display()), &owned)
    })
}

fn run_circleci_graph(file: &Path) -> Result<RunOutcome, AnalysisError> {
    let report = analyze_circleci(file)?;
    Ok(RunOutcome {
        output: circleci_graph(&report),
        exit_code: ExitCode::SUCCESS,
    })
}

fn run_dotenv_audit(
    files: &[PathBuf],
    format: OutputFormat,
    fail_on: FailLevel,
    ignores: &[String],
    config: &Option<PathBuf>,
) -> Result<RunOutcome, AnalysisError> {
    let rules = load_rules(config)?;
    let issues = envorigin::audit::audit_dotenv_files(files, rules.as_ref());
    exit_code_for_audit(&issues, fail_on, format, ignores, |filtered| {
        let owned: Vec<AuditIssue> = filtered.iter().map(|issue| (*issue).clone()).collect();
        envorigin::audit::audit_human(
            &format!(
                "files: {}",
                files
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            &owned,
        )
    })
}

fn select_service<'a>(
    report: &'a envorigin::model::ProjectReport,
    requested: Option<&'a str>,
) -> Result<&'a str, AnalysisError> {
    if let Some(service) = requested {
        if report
            .services
            .iter()
            .any(|candidate| candidate.name == service)
        {
            return Ok(service);
        }
        return Err(AnalysisError::UnknownService(service.to_string()));
    }
    if report.services.len() == 1 {
        return Ok(&report.services[0].name);
    }
    Err(AnalysisError::ServiceRequired(
        report
            .services
            .iter()
            .map(|service| service.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    ))
}
