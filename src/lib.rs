pub mod cli;
pub mod compose;
pub mod docker;
pub mod dotenv;
pub mod interpolation;
pub mod model;
pub mod output;

use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use compose::{analyze_service, load_compose};
use docker::DockerVerification;
use dotenv::parse_dotenv_file;
use interpolation::InterpolationContext;
use model::{AnalysisError, Diagnostic, ProjectReport, Severity, SourceKind, SourceRef};

#[derive(Debug, Clone)]
pub struct AnalyzeOptions {
    pub compose_file: PathBuf,
    pub env_files: Vec<PathBuf>,
    pub project_directory: Option<PathBuf>,
    pub host_env_file: Option<PathBuf>,
    pub docker_check: bool,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self {
            compose_file: PathBuf::from("compose.yaml"),
            env_files: Vec::new(),
            project_directory: None,
            host_env_file: None,
            docker_check: true,
        }
    }
}

pub fn analyze(options: &AnalyzeOptions) -> Result<ProjectReport, AnalysisError> {
    let compose_file = absolute(&options.compose_file)?;
    let compose = load_compose(&compose_file)?;
    let project_directory = match &options.project_directory {
        Some(path) => absolute(path)?,
        None => compose_file
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
    };

    let mut diagnostics = Vec::new();
    let mut context = InterpolationContext::new();

    let host_values: BTreeMap<String, String> = env::vars().collect();
    for (key, value) in host_values {
        context.define(
            key,
            Some(value),
            SourceRef::new(
                SourceKind::HostEnvironment,
                None,
                None,
                "process environment",
            ),
            10_000,
        );
    }

    if let Some(host_file) = &options.host_env_file {
        let path = absolute(host_file)?;
        let parsed = parse_dotenv_file(&path)?;
        diagnostics.extend(parsed.diagnostics);
        for entry in parsed.entries {
            let value = entry
                .value
                .as_deref()
                .map(|raw| context.interpolate_if(raw, entry.interpolate).value);
            context.define(
                entry.key,
                value,
                SourceRef::new(
                    SourceKind::HostEnvironment,
                    Some(path.clone()),
                    Some(entry.line),
                    "host environment overlay",
                ),
                20_000,
            );
        }
    }

    let interpolation_files = if options.env_files.is_empty() {
        let default = project_directory.join(".env");
        if default.exists() {
            vec![default]
        } else {
            Vec::new()
        }
    } else {
        options
            .env_files
            .iter()
            .map(|path| absolute(path))
            .collect::<Result<Vec<_>, _>>()?
    };

    for (index, path) in interpolation_files.iter().enumerate() {
        let parsed = parse_dotenv_file(path)?;
        diagnostics.extend(parsed.diagnostics);
        let kind = if options.env_files.is_empty() {
            SourceKind::DefaultEnvFile
        } else {
            SourceKind::CliEnvFile
        };
        for entry in parsed.entries {
            let interpolated = entry
                .value
                .as_deref()
                .map(|raw| context.interpolate_if(raw, entry.interpolate));
            if let Some(result) = &interpolated {
                diagnostics.extend(result.diagnostics.clone());
            }
            context.define(
                entry.key,
                interpolated.map(|result| result.value),
                SourceRef::new(
                    kind.clone(),
                    Some(path.clone()),
                    Some(entry.line),
                    if options.env_files.is_empty() {
                        "default Compose interpolation file"
                    } else {
                        "explicit Compose interpolation file"
                    },
                ),
                100 + index as i32,
            );
        }
    }

    if context.contains("COMPOSE_ENV_FILES") {
        diagnostics.push(Diagnostic::new(
            Severity::Warning,
            "unsupported-compose-env-files",
            "COMPOSE_ENV_FILES is set; v0.1 does not automatically expand that file list",
        ));
    }
    if context.contains("COMPOSE_FILE") {
        diagnostics.push(Diagnostic::new(
            Severity::Warning,
            "compose-file-redirect",
            "COMPOSE_FILE is set; use --file explicitly so the analyzed file is unambiguous",
        ));
    }

    let docker = if options.docker_check {
        DockerVerification::run(
            &compose_file,
            &options.env_files,
            &project_directory,
            options.host_env_file.as_deref(),
        )
    } else {
        DockerVerification::skipped()
    };

    diagnostics.extend(docker.diagnostics.clone());

    let mut services = Vec::new();
    for (name, service) in &compose.services {
        let canonical = docker.services.get(name);
        services.push(analyze_service(
            name,
            service,
            &compose_file,
            &context,
            canonical,
        )?);
    }

    Ok(ProjectReport {
        compose_file,
        project_directory,
        interpolation_files,
        docker_status: docker.status,
        services,
        diagnostics,
    })
}

fn absolute(path: &Path) -> Result<PathBuf, AnalysisError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(env::current_dir()?.join(path))
}
