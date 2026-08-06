use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnalysisError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid YAML in {path}: {source}")]
    Yaml {
        path: PathBuf,
        source: serde_yaml::Error,
    },
    #[error("invalid Compose configuration in {path}: {message}")]
    InvalidCompose { path: PathBuf, message: String },
    #[error("invalid dotenv file {path} at line {line}: {message}")]
    Dotenv {
        path: PathBuf,
        line: usize,
        message: String,
    },
    #[error("Compose file does not define any services: {0}")]
    NoServices(PathBuf),
    #[error("unknown service '{0}'")]
    UnknownService(String),
    #[error("--service is required because the Compose file defines multiple services: {0}")]
    ServiceRequired(String),
    #[error("variable '{variable}' is not configured for service '{service}'")]
    UnknownVariable { service: String, variable: String },
    #[error("invalid workflow in {path}: {message}")]
    InvalidWorkflow { path: PathBuf, message: String },
    #[error("invalid rules file {path}: {message}")]
    InvalidRules { path: PathBuf, message: String },
    #[error("unknown job '{0}'")]
    UnknownJob(String),
    #[error("--step is required because job '{0}' defines multiple steps")]
    StepRequired(String),
    #[error("unknown step in job '{job}': {step}")]
    UnknownStep { job: String, step: String },
    #[error("variable '{variable}' is not configured for job '{job}'")]
    UnknownActionsVariable { job: String, variable: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    HostEnvironment,
    DefaultEnvFile,
    CliEnvFile,
    ServiceEnvFile,
    ServiceEnvironment,
    DockerCanonical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRef {
    pub kind: SourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub detail: String,
}

impl SourceRef {
    pub fn new(
        kind: SourceKind,
        path: Option<PathBuf>,
        line: Option<usize>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            path,
            line,
            detail: detail.into(),
        }
    }

    pub fn label(&self) -> String {
        let base = self
            .path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| match self.kind {
                SourceKind::HostEnvironment => "shell".to_string(),
                SourceKind::DockerCanonical => "docker compose config".to_string(),
                _ => self.detail.clone(),
            });
        match self.line {
            Some(line) => format!("{base}:{line}"),
            None => base,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateDisposition {
    Winner,
    Shadowed,
    Derived,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub source: SourceRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub disposition: CandidateDisposition,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VariableState {
    Present,
    Absent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Explanation {
    pub variable: String,
    pub service: String,
    pub state: VariableState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<SourceRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_from: Vec<SourceRef>,
    pub candidates: Vec<Candidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceReport {
    pub name: String,
    pub variables: Vec<Explanation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectReport {
    pub compose_file: PathBuf,
    pub project_directory: PathBuf,
    pub interpolation_files: Vec<PathBuf>,
    pub docker_status: DockerStatus,
    pub services: Vec<ServiceReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl ProjectReport {
    pub fn explain(&self, service: &str, variable: &str) -> Result<&Explanation, AnalysisError> {
        let service_report = self
            .services
            .iter()
            .find(|candidate| candidate.name == service)
            .ok_or_else(|| AnalysisError::UnknownService(service.to_string()))?;
        service_report
            .variables
            .iter()
            .find(|candidate| candidate.variable == variable)
            .ok_or_else(|| AnalysisError::UnknownVariable {
                service: service.to_string(),
                variable: variable.to_string(),
            })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DockerStatus {
    Verified,
    Unavailable,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
}

impl Diagnostic {
    pub fn new(severity: Severity, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity,
            code: code.into(),
            message: message.into(),
        }
    }
}
