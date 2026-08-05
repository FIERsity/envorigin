//! Environment audit: turn analysis results into a checkable health report.
//!
//! `audit` collects existing diagnostics, then adds checks that a diagnostic
//! model cannot express: sensitive values in plaintext, dead-code env lines
//! (shadowed candidates with a different value), and interpolation variables
//! nobody consumes. Each issue carries a severity and a location, and
//! `--fail-on` turns the audit into a CI gate.

use std::collections::HashSet;
use std::path::PathBuf;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::actions::{ActionsDisposition, ActionsReport};
use crate::dotenv::parse_dotenv_file;
use crate::model::{CandidateDisposition, Diagnostic, ProjectReport, Severity};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditIssue {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

impl AuditIssue {
    pub fn new(
        severity: Severity,
        code: impl Into<String>,
        message: impl Into<String>,
        path: Option<PathBuf>,
        line: Option<usize>,
    ) -> Self {
        Self {
            severity,
            code: code.into(),
            message: message.into(),
            path,
            line,
        }
    }
}

impl From<&Diagnostic> for AuditIssue {
    fn from(diagnostic: &Diagnostic) -> Self {
        Self::new(
            diagnostic.severity,
            diagnostic.code.clone(),
            diagnostic.message.clone(),
            None,
            None,
        )
    }
}

/// Variable names that suggest they carry credentials or keys.
fn is_sensitive(name: &str) -> bool {
    let pattern = Regex::new(
        r"(?i)TOKEN|SECRET|PASSWORD|PASSWD|API.?KEY|ACCESS.?KEY|PRIVATE.?KEY|CREDENTIAL|AUTH|PWD",
    )
    .expect("valid regex");
    pattern.is_match(name)
}

fn is_expression(value: &str) -> bool {
    value.contains("${{") && value.contains("}}")
}

pub fn audit_project(report: &ProjectReport) -> Vec<AuditIssue> {
    let mut issues = Vec::new();
    for diagnostic in &report.diagnostics {
        issues.push(AuditIssue::from(diagnostic));
    }

    let mut consumed: HashSet<&str> = HashSet::new();
    for service in &report.services {
        for variable in &service.variables {
            consumed.insert(variable.variable.as_str());
            for diagnostic in &variable.diagnostics {
                issues.push(AuditIssue::from(diagnostic));
            }
            if let Some(value) = variable.value.as_deref() {
                if is_sensitive(&variable.variable) && !value.is_empty() {
                    let location = variable
                        .winner
                        .as_ref()
                        .map(|winner| (winner.path.clone(), winner.line));
                    issues.push(AuditIssue::new(
                        Severity::Error,
                        "sensitive-value",
                        format!(
                            "{} is set to a concrete value; verify it is not a real credential",
                            variable.variable
                        ),
                        location.as_ref().and_then(|(path, _)| path.clone()),
                        location.and_then(|(_, line)| line),
                    ));
                }
            }
            for candidate in &variable.candidates {
                if candidate.disposition == CandidateDisposition::Shadowed
                    && candidate.value.as_deref() != variable.value.as_deref()
                {
                    issues.push(AuditIssue::new(
                        Severity::Warning,
                        "shadowed-env-line",
                        format!(
                            "{} is shadowed by a higher-precedence layer and can be removed",
                            variable.variable
                        ),
                        candidate.source.path.clone(),
                        candidate.source.line,
                    ));
                }
            }
        }
    }

    for path in &report.interpolation_files {
        let Ok(parsed) = parse_dotenv_file(path) else {
            continue;
        };
        for entry in parsed.entries {
            if !consumed.contains(entry.key.as_str()) {
                issues.push(AuditIssue::new(
                    Severity::Info,
                    "unused-interpolation-variable",
                    format!(
                        "{} in the interpolation file is not consumed by any service",
                        entry.key
                    ),
                    Some(path.clone()),
                    Some(entry.line),
                ));
            }
        }
    }
    issues
}

fn deduplicate(issues: Vec<AuditIssue>) -> Vec<AuditIssue> {
    let mut seen = std::collections::HashSet::new();
    issues
        .into_iter()
        .filter(|issue| {
            seen.insert((
                issue.code.clone(),
                issue.message.clone(),
                issue.path.clone(),
                issue.line,
            ))
        })
        .collect()
}

pub fn audit_workflow(report: &ActionsReport) -> Vec<AuditIssue> {
    let mut issues = Vec::new();
    for diagnostic in &report.diagnostics {
        issues.push(AuditIssue::from(diagnostic));
    }
    for job in &report.jobs {
        for variable in job
            .variables
            .iter()
            .chain(job.steps.iter().flat_map(|step| step.variables.iter()))
        {
            for diagnostic in &variable.diagnostics {
                issues.push(AuditIssue::from(diagnostic));
            }
            if let Some(value) = variable.value.as_deref() {
                if is_sensitive(&variable.variable) && !value.is_empty() && !is_expression(value) {
                    let location = variable
                        .winner
                        .as_ref()
                        .map(|winner| (winner.path.clone(), winner.line));
                    issues.push(AuditIssue::new(
                        Severity::Error,
                        "sensitive-value",
                        format!(
                            "{} is set to a concrete value; verify it is not a real credential",
                            variable.variable
                        ),
                        location.as_ref().and_then(|(path, _)| path.clone()),
                        location.and_then(|(_, line)| line),
                    ));
                }
            }
            for candidate in &variable.candidates {
                if candidate.disposition == ActionsDisposition::Shadowed
                    && candidate.value.as_deref() != variable.value.as_deref()
                {
                    issues.push(AuditIssue::new(
                        Severity::Warning,
                        "shadowed-env-line",
                        format!(
                            "{} is shadowed by a higher-precedence layer and can be removed",
                            variable.variable
                        ),
                        candidate.source.path.clone(),
                        candidate.source.line,
                    ));
                }
            }
        }
    }
    deduplicate(issues)
}

// --- rendering -----------------------------------------------------------

pub fn audit_human(target: &str, issues: &[AuditIssue]) -> String {
    let mut output = String::new();
    use std::fmt::Write;
    let _ = writeln!(output, "EnvOrigin {} — audit", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(output, "{target}");
    let count = |severity: Severity| {
        issues
            .iter()
            .filter(|issue| issue.severity == severity)
            .count()
    };
    let _ = writeln!(
        output,
        "\n{} error(s), {} warning(s), {} info",
        count(Severity::Error),
        count(Severity::Warning),
        count(Severity::Info)
    );
    if issues.is_empty() {
        let _ = writeln!(output, "no issues found");
        return output;
    }
    for issue in issues {
        let location = match (&issue.path, issue.line) {
            (Some(path), Some(line)) => format!("{}:{}", path.display(), line),
            (Some(path), None) => path.display().to_string(),
            (None, _) => String::new(),
        };
        let _ = writeln!(
            output,
            "\n{} [{}]: {}{}",
            severity_label(issue.severity),
            issue.code,
            issue.message,
            if location.is_empty() {
                String::new()
            } else {
                format!(" ({location})")
            }
        );
    }
    output
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}
