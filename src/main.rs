use std::process::ExitCode;

use clap::Parser;
use envorigin::cli::{Cli, Command, CommonArgs, OutputFormat};
use envorigin::model::AnalysisError;
use envorigin::output::{
    explanation_human, explanation_json, project_human, project_json, service_human,
};
use envorigin::{analyze, AnalyzeOptions};

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<String, AnalysisError> {
    match cli.command {
        Command::Scan(args) => {
            let report = analyze(&options(&args.common))?;
            if let Some(service_name) = args.service {
                let service = report
                    .services
                    .iter()
                    .find(|service| service.name == service_name)
                    .ok_or_else(|| AnalysisError::UnknownService(service_name.clone()))?;
                Ok(match args.common.format {
                    OutputFormat::Human => service_human(&report, service, args.common.show_values),
                    OutputFormat::Json => {
                        let mut filtered = report.clone();
                        filtered.services = vec![service.clone()];
                        project_json(&filtered, args.common.show_values)
                    }
                })
            } else {
                Ok(match args.common.format {
                    OutputFormat::Human => project_human(&report, args.common.show_values),
                    OutputFormat::Json => project_json(&report, args.common.show_values),
                })
            }
        }
        Command::Explain(args) => {
            let report = analyze(&options(&args.common))?;
            let service = select_service(&report, args.service.as_deref())?;
            let explanation = report.explain(service, &args.variable)?;
            Ok(match args.common.format {
                OutputFormat::Human => explanation_human(explanation, args.common.show_values),
                OutputFormat::Json => explanation_json(explanation, args.common.show_values),
            })
        }
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
