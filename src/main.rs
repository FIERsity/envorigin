use std::fmt::Write as _;
use std::io::{ErrorKind, Write};
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
            let rules = match load_rules(&args.config) {
                Ok(rules) => rules,
                Err(error) => {
                    eprintln!("error: {error}");
                    return ExitCode::FAILURE;
                }
            };
            let issues = envorigin::audit::audit_dotenv_files(&args.files, rules.as_ref());
            return match exit_code_for_audit(&issues, args.fail_on, args.format, &[], |filtered| {
                let owned: Vec<AuditIssue> =
                    filtered.iter().map(|issue| (*issue).clone()).collect();
                envorigin::audit::audit_human(
                    &format!(
                        "files: {}",
                        args.files
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    &owned,
                )
            }) {
                Ok(outcome) => {
                    println!("{}", outcome.output);
                    outcome.exit_code
                }
                Err(_) => ExitCode::FAILURE,
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
        Command::Scan(args) => {
            let report = analyze(&options(&args.common))?;
            if let Some(service_name) = args.service {
                let service = report
                    .services
                    .iter()
                    .find(|service| service.name == service_name)
                    .ok_or_else(|| AnalysisError::UnknownService(service_name.clone()))?;
                Ok(outcome(match args.common.format {
                    OutputFormat::Human => service_human(&report, service, args.common.show_values),
                    OutputFormat::Github => {
                        service_human(&report, service, args.common.show_values)
                    }
                    OutputFormat::Json => {
                        let mut filtered = report.clone();
                        filtered.services = vec![service.clone()];
                        project_json(&filtered, args.common.show_values)
                    }
                }))
            } else {
                Ok(outcome(match args.common.format {
                    OutputFormat::Human => project_human(&report, args.common.show_values),
                    OutputFormat::Github => project_human(&report, args.common.show_values),
                    OutputFormat::Json => project_json(&report, args.common.show_values),
                }))
            }
        }
        Command::Explain(args) => {
            let report = analyze(&options(&args.common))?;
            let service = select_service(&report, args.service.as_deref())?;
            let explanation = report.explain(service, &args.variable)?;
            let mut output = match args.common.format {
                OutputFormat::Human => explanation_human(explanation, args.common.show_values),
                OutputFormat::Github => explanation_human(explanation, args.common.show_values),
                OutputFormat::Json => explanation_json(explanation, args.common.show_values),
            };
            if args.debug {
                output = format!(
                    "{}\n{}",
                    debug_trace(&report, service, &args.variable),
                    output
                );
            }
            Ok(outcome(output))
        }
        Command::Audit(args) => {
            let report = analyze(&options(&args.common))?;
            let rules = load_rules(&args.config)?;
            let issues = audit_project(&report, rules.as_ref());
            exit_code_for_audit(
                &issues,
                args.fail_on,
                args.common.format,
                &args.ignore,
                |filtered| {
                    let owned: Vec<AuditIssue> =
                        filtered.iter().map(|issue| (*issue).clone()).collect();
                    audit_human(
                        &format!("compose: {}", report.compose_file.display()),
                        &owned,
                    )
                },
            )
        }
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
        Command::Graph(args) => {
            let report = analyze(&options(&args.common))?;
            Ok(outcome(project_graph(&report)))
        }
        Command::Circleci(args) => match args.command {
            CircleciCommand::Scan(scan) => {
                let report = analyze_circleci(&scan.file)?;
                Ok(outcome(match scan.format {
                    OutputFormat::Human => circleci_human(&report, scan.show_values),
                    OutputFormat::Github => circleci_human(&report, scan.show_values),
                    OutputFormat::Json => circleci_json(&report, scan.show_values),
                }))
            }
            CircleciCommand::Explain(explain) => {
                let report = analyze_circleci(&explain.common.file)?;
                let variable = report.explain(&explain.job, &explain.variable)?;
                Ok(outcome(match explain.common.format {
                    OutputFormat::Human => {
                        circleci_variable_human(&report, variable, explain.common.show_values)
                    }
                    OutputFormat::Github => {
                        circleci_variable_human(&report, variable, explain.common.show_values)
                    }
                    OutputFormat::Json => {
                        circleci_variable_json(&report, variable, explain.common.show_values)
                    }
                }))
            }
            CircleciCommand::Graph(graph) => {
                let report = analyze_circleci(&graph.file)?;
                Ok(outcome(circleci_graph(&report)))
            }
            CircleciCommand::Audit(audit) => {
                let report = analyze_circleci(&audit.common.file)?;
                let rules = load_rules(&audit.config)?;
                let issues = audit_circleci(&report, rules.as_ref());
                exit_code_for_audit(
                    &issues,
                    audit.fail_on,
                    audit.common.format,
                    &audit.ignore,
                    |filtered| {
                        let owned: Vec<AuditIssue> =
                            filtered.iter().map(|issue| (*issue).clone()).collect();
                        audit_human(&format!("file: {}", report.file.display()), &owned)
                    },
                )
            }
        },
        Command::Gitlab(args) => match args.command {
            GitlabCommand::Scan(scan) => {
                let report = analyze_gitlab(&scan.file)?;
                Ok(outcome(match scan.format {
                    OutputFormat::Human => gitlab_human(&report, scan.show_values),
                    OutputFormat::Github => gitlab_human(&report, scan.show_values),
                    OutputFormat::Json => gitlab_json(&report, scan.show_values),
                }))
            }
            GitlabCommand::Explain(explain) => {
                let report = analyze_gitlab(&explain.common.file)?;
                let variable = report.explain(explain.job.as_deref(), &explain.variable)?;
                Ok(outcome(match explain.common.format {
                    OutputFormat::Human => {
                        gitlab_variable_human(&report, variable, explain.common.show_values)
                    }
                    OutputFormat::Github => {
                        gitlab_variable_human(&report, variable, explain.common.show_values)
                    }
                    OutputFormat::Json => {
                        gitlab_variable_json(&report, variable, explain.common.show_values)
                    }
                }))
            }
            GitlabCommand::Graph(graph) => {
                let report = analyze_gitlab(&graph.file)?;
                Ok(outcome(gitlab_graph(&report)))
            }
            GitlabCommand::Audit(audit) => {
                let report = analyze_gitlab(&audit.common.file)?;
                let rules = load_rules(&audit.config)?;
                let issues = audit_gitlab(&report, rules.as_ref());
                exit_code_for_audit(
                    &issues,
                    audit.fail_on,
                    audit.common.format,
                    &audit.ignore,
                    |filtered| {
                        let owned: Vec<AuditIssue> =
                            filtered.iter().map(|issue| (*issue).clone()).collect();
                        audit_human(&format!("file: {}", report.file.display()), &owned)
                    },
                )
            }
        },
        // Handled in main() before run() is called.
        Command::Lsp => Ok(outcome(String::new())),
        Command::Init => Ok(outcome(String::new())),
        Command::Dotenv(_) => Ok(outcome(String::new())),
        Command::Completions(_) => Ok(outcome(String::new())),
        Command::Actions(args) => match args.command {
            ActionsCommand::Scan(scan) => {
                let report = envorigin::actions::analyze_workflow(
                    &scan.workflow_file,
                    scan.project_directory.as_deref(),
                )?;
                Ok(outcome(match scan.format {
                    OutputFormat::Human => actions_human(&report, scan.show_values),
                    OutputFormat::Github => actions_human(&report, scan.show_values),
                    OutputFormat::Json => actions_json(&report, scan.show_values),
                }))
            }
            ActionsCommand::Explain(explain) => {
                let report = envorigin::actions::analyze_workflow(
                    &explain.common.workflow_file,
                    explain.common.project_directory.as_deref(),
                )?;
                let job = select_job(&report, explain.job.as_deref())?;
                let step_index = match &explain.step {
                    None => None,
                    Some(spec) => Some(select_step(&report, job, spec)?),
                };
                let variable = report.explain(job, step_index, &explain.variable)?;
                let mut output = match explain.common.format {
                    OutputFormat::Human => {
                        actions_variable_human(&report, variable, explain.common.show_values)
                    }
                    OutputFormat::Github => {
                        actions_variable_human(&report, variable, explain.common.show_values)
                    }
                    OutputFormat::Json => {
                        actions_variable_json(&report, variable, explain.common.show_values)
                    }
                };
                if explain.debug && !matches!(explain.common.format, OutputFormat::Json) {
                    output = format!(
                        "{}\n{}",
                        envorigin::actions::actions_debug_trace(variable),
                        output
                    );
                }
                Ok(outcome(output))
            }
            ActionsCommand::Audit(audit) => {
                let report = envorigin::actions::analyze_workflow(
                    &audit.common.workflow_file,
                    audit.common.project_directory.as_deref(),
                )?;
                let rules = load_rules(&audit.config)?;
                let issues = audit_workflow(&report, rules.as_ref());
                exit_code_for_audit(
                    &issues,
                    audit.fail_on,
                    audit.common.format,
                    &audit.ignore,
                    |filtered| {
                        let owned: Vec<AuditIssue> =
                            filtered.iter().map(|issue| (*issue).clone()).collect();
                        audit_human(
                            &format!("workflow: {}", report.workflow_file.display()),
                            &owned,
                        )
                    },
                )
            }
            ActionsCommand::Graph(graph) => {
                let report = envorigin::actions::analyze_workflow(
                    &graph.workflow_file,
                    graph.project_directory.as_deref(),
                )?;
                Ok(outcome(actions_graph(&report)))
            }
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

fn options(args: &CommonArgs) -> AnalyzeOptions {
    AnalyzeOptions {
        compose_file: args.compose_file.clone(),
        env_files: args.env_files.clone(),
        project_directory: args.project_directory.clone(),
        host_env_file: args.host_env_file.clone(),
        docker_check: !args.no_docker_check,
    }
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
