use std::process::ExitCode;

use clap::Parser;
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
use envorigin::diff::{diff_files, diff_human, diff_json};
use envorigin::gitlab::{
    analyze_gitlab, gitlab_human, gitlab_json, gitlab_variable_human, gitlab_variable_json,
};
use envorigin::graph::{actions_graph, project_graph};
use envorigin::model::AnalysisError;
use envorigin::output::{
    explanation_human, explanation_json, project_human, project_json, service_human,
};
use envorigin::rules::{default_config_path, Rules};
use envorigin::{analyze, AnalyzeOptions};

struct RunOutcome {
    output: String,
    exit_code: ExitCode,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if matches!(&cli.command, Command::Lsp) {
        envorigin::lsp::run_lsp();
        return ExitCode::SUCCESS;
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
    let outcome = |output: String| RunOutcome {
        output,
        exit_code: ExitCode::SUCCESS,
    };
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
                    OutputFormat::Json => {
                        let mut filtered = report.clone();
                        filtered.services = vec![service.clone()];
                        project_json(&filtered, args.common.show_values)
                    }
                }))
            } else {
                Ok(outcome(match args.common.format {
                    OutputFormat::Human => project_human(&report, args.common.show_values),
                    OutputFormat::Json => project_json(&report, args.common.show_values),
                }))
            }
        }
        Command::Explain(args) => {
            let report = analyze(&options(&args.common))?;
            let service = select_service(&report, args.service.as_deref())?;
            let explanation = report.explain(service, &args.variable)?;
            Ok(outcome(match args.common.format {
                OutputFormat::Human => explanation_human(explanation, args.common.show_values),
                OutputFormat::Json => explanation_json(explanation, args.common.show_values),
            }))
        }
        Command::Audit(args) => {
            let report = analyze(&options(&args.common))?;
            let rules = load_rules(&args.config)?;
            let issues = audit_project(&report, rules.as_ref());
            exit_code_for_audit(
                audit_human(
                    &format!("compose: {}", report.compose_file.display()),
                    &issues,
                ),
                &issues,
                args.fail_on,
            )
        }
        Command::Diff(args) => {
            let report = diff_files(&args.files)?;
            Ok(outcome(match args.format {
                OutputFormat::Human => diff_human(&report, args.show_values),
                OutputFormat::Json => diff_json(&report, args.show_values),
            }))
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
                    OutputFormat::Json => {
                        circleci_variable_json(&report, variable, explain.common.show_values)
                    }
                }))
            }
            CircleciCommand::Audit(audit) => {
                let report = analyze_circleci(&audit.common.file)?;
                let rules = load_rules(&audit.config)?;
                let issues = audit_circleci(&report, rules.as_ref());
                exit_code_for_audit(
                    audit_human(&format!("file: {}", report.file.display()), &issues),
                    &issues,
                    audit.fail_on,
                )
            }
        },
        Command::Gitlab(args) => match args.command {
            GitlabCommand::Scan(scan) => {
                let report = analyze_gitlab(&scan.file)?;
                Ok(outcome(match scan.format {
                    OutputFormat::Human => gitlab_human(&report, scan.show_values),
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
                    OutputFormat::Json => {
                        gitlab_variable_json(&report, variable, explain.common.show_values)
                    }
                }))
            }
            GitlabCommand::Audit(audit) => {
                let report = analyze_gitlab(&audit.common.file)?;
                let rules = load_rules(&audit.config)?;
                let issues = audit_gitlab(&report, rules.as_ref());
                exit_code_for_audit(
                    audit_human(&format!("file: {}", report.file.display()), &issues),
                    &issues,
                    audit.fail_on,
                )
            }
        },
        // Handled in main() before run() is called.
        Command::Lsp => Ok(outcome(String::new())),
        Command::Actions(args) => match args.command {
            ActionsCommand::Scan(scan) => {
                let report = envorigin::actions::analyze_workflow(
                    &scan.workflow_file,
                    scan.project_directory.as_deref(),
                )?;
                Ok(outcome(match scan.format {
                    OutputFormat::Human => actions_human(&report, scan.show_values),
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
                Ok(outcome(match explain.common.format {
                    OutputFormat::Human => {
                        actions_variable_human(&report, variable, explain.common.show_values)
                    }
                    OutputFormat::Json => {
                        actions_variable_json(&report, variable, explain.common.show_values)
                    }
                }))
            }
            ActionsCommand::Audit(audit) => {
                let report = envorigin::actions::analyze_workflow(
                    &audit.common.workflow_file,
                    audit.common.project_directory.as_deref(),
                )?;
                let rules = load_rules(&audit.config)?;
                let issues = audit_workflow(&report, rules.as_ref());
                exit_code_for_audit(
                    audit_human(
                        &format!("workflow: {}", report.workflow_file.display()),
                        &issues,
                    ),
                    &issues,
                    audit.fail_on,
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
    output: String,
    issues: &[AuditIssue],
    fail_on: FailLevel,
) -> Result<RunOutcome, AnalysisError> {
    let failed = issues.iter().any(|issue| fail_on.triggers(issue.severity));
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
