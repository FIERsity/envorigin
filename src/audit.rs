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
pub(crate) fn is_sensitive(name: &str) -> bool {
    let pattern = Regex::new(
        r"(?i)TOKEN|SECRET|PASSWORD|PASSWD|API.?KEY|ACCESS.?KEY|PRIVATE.?KEY|CREDENTIAL|AUTH|PWD",
    )
    .expect("valid regex");
    pattern.is_match(name)
}

/// Values that are clearly placeholders or examples rather than real
/// credentials (e.g. `wordpress`, `changeit`, `somewordpress`).
fn is_placeholder_value(value: &str) -> bool {
    let lower = value.to_lowercase();
    if lower.chars().all(|ch| ch.is_ascii_lowercase()) && lower.len() < 16 {
        return true;
    }
    let placeholder_pattern = Regex::new(
        r"(?i)changeme|changeit|example|dummy|foobar|qwerty|letmein|secret|password|your_|xxxx",
    )
    .expect("valid regex");
    placeholder_pattern.is_match(value)
}

/// Values that reference an external secret manager rather than carrying a
/// concrete secret (Vault, AWS Secrets Manager/SSM, secret templating).
fn secret_manager_ref(value: &str) -> Option<&'static str> {
    let lower = value.to_lowercase();
    if lower.starts_with("vault:") {
        Some("HashiCorp Vault")
    } else if lower.starts_with("arn:aws:secretsmanager:") {
        Some("AWS Secrets Manager")
    } else if lower.starts_with("arn:aws:ssm:") || lower.starts_with("sm://") {
        Some("AWS SSM")
    } else if value.contains("{{ secret") || value.contains("{{secrets.") {
        Some("secret templating")
    } else {
        None
    }
}

fn is_expression(value: &str) -> bool {
    value.contains("${{") && value.contains("}}")
}

fn check_sensitive_value(
    issues: &mut Vec<AuditIssue>,
    name: &str,
    value: &str,
    path: Option<PathBuf>,
    line: Option<usize>,
) {
    if let Some(provider) = secret_manager_ref(value) {
        issues.push(AuditIssue::new(
            Severity::Info,
            "secret-manager-reference",
            format!(
                "{name} references an external secret ({provider}); its value cannot be resolved statically"
            ),
            path,
            line,
        ));
    } else if is_placeholder_value(value) {
        issues.push(AuditIssue::new(
            Severity::Warning,
            "sensitive-placeholder",
            format!("{name} is set to a placeholder-looking value; replace it before deploying"),
            path,
            line,
        ));
    } else {
        issues.push(AuditIssue::new(
            Severity::Error,
            "sensitive-value",
            format!("{name} is set to a concrete value; verify it is not a real credential"),
            path,
            line,
        ));
    }
}

pub fn audit_project(
    report: &ProjectReport,
    rules: Option<&crate::rules::Rules>,
) -> Vec<AuditIssue> {
    let mut issues = Vec::new();
    for diagnostic in &report.diagnostics {
        issues.push(AuditIssue::from(diagnostic));
    }

    let mut consumed: HashSet<&str> = HashSet::new();
    // Original file texts: interpolated values lose the expression text, so
    // reference detection must look at the raw sources, not the final values.
    let mut source_files: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    source_files.insert(report.compose_file.clone());
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
                    check_sensitive_value(
                        &mut issues,
                        &variable.variable,
                        value,
                        location.as_ref().and_then(|(path, _)| path.clone()),
                        location.and_then(|(_, line)| line),
                    );
                }
            }
            for candidate in &variable.candidates {
                if let Some(path) = &candidate.source.path {
                    source_files.insert(path.clone());
                }
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
    let file_texts: Vec<String> = source_files
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap_or_default())
        .collect();

    for path in &report.interpolation_files {
        let Ok(parsed) = parse_dotenv_file(path) else {
            continue;
        };
        for entry in parsed.entries {
            let referenced = file_texts.iter().any(|text| {
                text.contains(&format!("${{{}}}", entry.key))
                    || text.contains(&format!("${}", entry.key))
            });
            if !consumed.contains(entry.key.as_str()) && !referenced {
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
    if let Some(rules) = rules {
        let variables: Vec<(String, Option<String>)> = report
            .services
            .iter()
            .flat_map(|service| service.variables.iter())
            .map(|variable| (variable.variable.clone(), variable.value.clone()))
            .collect();
        issues.extend(crate::rules::check_rules(&variables, rules));
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

pub fn audit_workflow(
    report: &ActionsReport,
    rules: Option<&crate::rules::Rules>,
) -> Vec<AuditIssue> {
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
                    check_sensitive_value(
                        &mut issues,
                        &variable.variable,
                        value,
                        location.as_ref().and_then(|(path, _)| path.clone()),
                        location.and_then(|(_, line)| line),
                    );
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
    if let Some(rules) = rules {
        let mut variables: Vec<(String, Option<String>)> = report
            .jobs
            .iter()
            .flat_map(|job| {
                job.variables
                    .iter()
                    .chain(job.steps.iter().flat_map(|step| step.variables.iter()))
                    .map(|variable| (variable.variable.clone(), variable.value.clone()))
            })
            .collect();
        variables.sort();
        variables.dedup();
        issues.extend(crate::rules::check_rules(&variables, rules));
    }
    deduplicate(issues)
}

pub fn audit_gitlab(
    report: &crate::gitlab::GitlabReport,
    rules: Option<&crate::rules::Rules>,
) -> Vec<AuditIssue> {
    let mut issues = Vec::new();
    for diagnostic in &report.diagnostics {
        issues.push(AuditIssue::from(diagnostic));
    }
    let mut seen = std::collections::HashSet::new();
    for variable in report
        .global_variables
        .iter()
        .chain(report.jobs.iter().flat_map(|job| job.variables.iter()))
    {
        audit_variable(&mut issues, variable, &mut seen);
    }
    if let Some(rules) = rules {
        let variables: Vec<(String, Option<String>)> =
            seen.iter().map(|name| (name.clone(), None)).collect();
        issues.extend(crate::rules::check_rules(&variables, rules));
    }
    deduplicate(issues)
}

pub fn audit_circleci(
    report: &crate::circleci::CircleciReport,
    rules: Option<&crate::rules::Rules>,
) -> Vec<AuditIssue> {
    let mut issues = Vec::new();
    for diagnostic in &report.diagnostics {
        issues.push(AuditIssue::from(diagnostic));
    }
    let mut seen = std::collections::HashSet::new();
    for variable in report.jobs.iter().flat_map(|job| job.variables.iter()) {
        audit_variable(&mut issues, variable, &mut seen);
    }
    if let Some(rules) = rules {
        let variables: Vec<(String, Option<String>)> =
            seen.iter().map(|name| (name.clone(), None)).collect();
        issues.extend(crate::rules::check_rules(&variables, rules));
    }
    deduplicate(issues)
}

/// Shared per-variable audit for the GitLab and CircleCI backends: variable
/// diagnostics, sensitive values, and shadowed dead-code lines. `seen` keeps
/// one report per variable name across the global + job scopes.
fn audit_variable<V>(issues: &mut Vec<AuditIssue>, variable: &V, seen: &mut HashSet<String>)
where
    V: VariableAudit,
{
    if !seen.insert(variable.name().to_string()) {
        return;
    }
    for diagnostic in variable.diagnostics() {
        issues.push(AuditIssue::from(diagnostic));
    }
    if let Some(value) = variable.value() {
        if is_sensitive(variable.name()) && !value.is_empty() {
            let location = variable.winner_location();
            check_sensitive_value(
                issues,
                variable.name(),
                value,
                location.as_ref().and_then(|(path, _)| path.clone()),
                location.and_then(|(_, line)| line),
            );
        }
    }
    for (path, line) in variable.shadowed_lines() {
        issues.push(AuditIssue::new(
            Severity::Warning,
            "shadowed-env-line",
            format!(
                "{} is shadowed by a higher-precedence layer and can be removed",
                variable.name()
            ),
            path,
            line,
        ));
    }
}

/// Abstraction over GitLab/CircleCI variable shapes so the shared audit
/// logic stays single-sourced.
trait VariableAudit {
    fn name(&self) -> &str;
    fn value(&self) -> Option<&str>;
    fn diagnostics(&self) -> &[Diagnostic];
    fn winner_location(&self) -> Option<(Option<PathBuf>, Option<usize>)>;
    fn shadowed_lines(&self) -> Vec<(Option<PathBuf>, Option<usize>)>;
}

impl VariableAudit for crate::gitlab::GitlabVariable {
    fn name(&self) -> &str {
        &self.variable
    }
    fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
    fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
    fn winner_location(&self) -> Option<(Option<PathBuf>, Option<usize>)> {
        self.winner
            .as_ref()
            .map(|winner| (winner.path.clone(), winner.line))
    }
    fn shadowed_lines(&self) -> Vec<(Option<PathBuf>, Option<usize>)> {
        self.candidates
            .iter()
            .filter(|candidate| {
                candidate.disposition == crate::gitlab::GitlabDisposition::Shadowed
                    && candidate.value.as_deref() != self.value.as_deref()
            })
            .map(|candidate| (candidate.source.path.clone(), candidate.source.line))
            .collect()
    }
}

impl VariableAudit for crate::circleci::CircleciVariable {
    fn name(&self) -> &str {
        &self.variable
    }
    fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
    fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
    fn winner_location(&self) -> Option<(Option<PathBuf>, Option<usize>)> {
        self.winner
            .as_ref()
            .map(|winner| (winner.path.clone(), winner.line))
    }
    fn shadowed_lines(&self) -> Vec<(Option<PathBuf>, Option<usize>)> {
        self.candidates
            .iter()
            .filter(|candidate| {
                candidate.disposition == crate::circleci::CircleciDisposition::Shadowed
                    && candidate.value.as_deref() != self.value.as_deref()
            })
            .map(|candidate| (candidate.source.path.clone(), candidate.source.line))
            .collect()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_name_detection() {
        assert!(is_sensitive("API_TOKEN"));
        assert!(is_sensitive("DB_PASSWORD"));
        assert!(is_sensitive("aws_access_key_id"));
        assert!(is_sensitive("private_key_path"));
        assert!(!is_sensitive("DATABASE_URL"));
        assert!(!is_sensitive("APP_NAME"));
    }

    #[test]
    fn placeholder_detection_boundaries() {
        // Placeholder-looking values downgrade the report.
        assert!(is_placeholder_value("wordpress"));
        assert!(is_placeholder_value("somewordpress"));
        assert!(is_placeholder_value("changeit"));
        assert!(is_placeholder_value("changeme"));
        // Real-looking values with digits/mixed case are NOT placeholders —
        // regression for the sk-live-12345 false positive.
        assert!(!is_placeholder_value("sk-live-12345"));
        assert!(!is_placeholder_value("Tr0ub4dor&3"));
        assert!(!is_placeholder_value("hf_0123456789abcdef"));
    }

    #[test]
    fn secret_manager_reference_detection() {
        assert_eq!(
            secret_manager_ref("vault:secret/data/db#password"),
            Some("HashiCorp Vault")
        );
        assert_eq!(
            secret_manager_ref("arn:aws:secretsmanager:us-east-1:123:secret:db"),
            Some("AWS Secrets Manager")
        );
        assert_eq!(secret_manager_ref("sm://prod/db"), Some("AWS SSM"));
        assert_eq!(
            secret_manager_ref("{{ secret.DEPLOY_KEY }}"),
            Some("secret templating")
        );
        assert_eq!(secret_manager_ref("plain-value"), None);
    }

    #[test]
    fn sensitive_checks_emit_the_right_severity() {
        let mut issues = Vec::new();
        check_sensitive_value(
            &mut issues,
            "TOKEN",
            "sk-live-12345",
            Some(PathBuf::from("compose.yaml")),
            Some(9),
        );
        check_sensitive_value(
            &mut issues,
            "PASSWORD",
            "changeit",
            Some(PathBuf::from("compose.yaml")),
            Some(10),
        );
        check_sensitive_value(
            &mut issues,
            "VAULT_TOKEN",
            "vault:secret/data/db#password",
            None,
            None,
        );
        assert_eq!(issues.len(), 3);
        assert_eq!(issues[0].code, "sensitive-value");
        assert_eq!(issues[0].severity, Severity::Error);
        assert_eq!(issues[1].code, "sensitive-placeholder");
        assert_eq!(issues[1].severity, Severity::Warning);
        assert_eq!(issues[2].code, "secret-manager-reference");
        assert_eq!(issues[2].severity, Severity::Info);
    }
}
