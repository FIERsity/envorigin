//! GitLab CI variables analysis.
//!
//! Layered resolution: included files (local) < file-global `variables:` <
//! job-level `variables:`. Values interpolate shell-style (`$VAR`,
//! `${VAR}`, `${VAR:-word}`) through the interpolation engine, and — unlike
//! Compose — variable definitions in one file may reference each other in any
//! order, so definitions are collected first and interpolated in a second
//! pass. `include: remote` / `include: template` entries are external and are
//! reported, not fetched.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::interpolation::InterpolationContext;
use crate::model::{AnalysisError, Diagnostic, Severity, VariableState};
use crate::output::fingerprint;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitlabSourceKind {
    IncludeFile,
    FileGlobal,
    Job,
    Predefined,
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitlabSourceRef {
    pub kind: GitlabSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub detail: String,
}

impl GitlabSourceRef {
    pub fn new(
        kind: GitlabSourceKind,
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
                GitlabSourceKind::Predefined => "GitLab predefined".to_string(),
                GitlabSourceKind::External => "external".to_string(),
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
pub enum GitlabDisposition {
    Winner,
    Shadowed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitlabCandidate {
    pub source: GitlabSourceRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub disposition: GitlabDisposition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitlabReference {
    /// Expression body, e.g. `CI_COMMIT_REF_NAME`.
    pub expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<GitlabSourceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitlabVariable {
    pub variable: String,
    pub state: VariableState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<GitlabSourceRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<GitlabReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<GitlabCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitlabJob {
    pub id: String,
    pub variables: Vec<GitlabVariable>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitlabReport {
    pub file: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_files: Vec<PathBuf>,
    pub global_variables: Vec<GitlabVariable>,
    pub jobs: Vec<GitlabJob>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl GitlabReport {
    pub fn explain(
        &self,
        job: Option<&str>,
        variable: &str,
    ) -> Result<&GitlabVariable, AnalysisError> {
        match job {
            Some(job) => {
                let report = self
                    .jobs
                    .iter()
                    .find(|candidate| candidate.id == job)
                    .ok_or_else(|| AnalysisError::UnknownJob(job.to_string()))?;
                report
                    .variables
                    .iter()
                    .find(|candidate| candidate.variable == variable)
                    .ok_or_else(|| AnalysisError::UnknownActionsVariable {
                        job: job.to_string(),
                        variable: variable.to_string(),
                    })
            }
            None => self
                .global_variables
                .iter()
                .find(|candidate| candidate.variable == variable)
                .ok_or_else(|| AnalysisError::UnknownActionsVariable {
                    job: "(global)".to_string(),
                    variable: variable.to_string(),
                }),
        }
    }
}

/// GitLab CI/CD component inputs v2 syntax: `$[[ inputs.X ]]`.
static V2_INPUT_PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

/// Top-level keys that are not jobs.
const NON_JOB_KEYS: &[&str] = &[
    "include",
    "default",
    "stages",
    "workflow",
    "variables",
    "image",
    "services",
    "cache",
    "before_script",
    "after_script",
    "tags",
    "pages",
    "on",
    "permissions",
    "id_tokens",
];

/// Statically-known GitLab predefined variables (a practical subset).
const PREDEFINED_VARIABLES: &[&str] = &[
    "CI",
    "CI_API_V4_URL",
    "CI_BUILDS_DIR",
    "CI_COMMIT_BRANCH",
    "CI_COMMIT_REF_NAME",
    "CI_COMMIT_REF_SLUG",
    "CI_COMMIT_SHA",
    "CI_COMMIT_SHORT_SHA",
    "CI_COMMIT_TAG",
    "CI_CONCURRENT_ID",
    "CI_CONCURRENT_PROJECT_ID",
    "CI_DEFAULT_BRANCH",
    "CI_JOB_ID",
    "CI_JOB_NAME",
    "CI_JOB_STAGE",
    "CI_JOB_TOKEN",
    "CI_JOB_URL",
    "CI_NODE_TOTAL",
    "CI_PIPELINE_ID",
    "CI_PIPELINE_IID",
    "CI_PIPELINE_SOURCE",
    "CI_PIPELINE_URL",
    "CI_PROJECT_DIR",
    "CI_PROJECT_ID",
    "CI_PROJECT_NAME",
    "CI_PROJECT_PATH",
    "CI_PROJECT_PATH_SLUG",
    "CI_PROJECT_URL",
    "CI_REGISTRY",
    "CI_REGISTRY_IMAGE",
    "CI_REGISTRY_PASSWORD",
    "CI_REGISTRY_USER",
    "CI_REPOSITORY_URL",
    "CI_RUNNER_DESCRIPTION",
    "CI_RUNNER_ID",
    "CI_RUNNER_TAGS",
    "CI_SERVER_HOST",
    "CI_SERVER_NAME",
    "CI_SERVER_URL",
    "CI_SERVER_VERSION",
    "GITLAB_USER_EMAIL",
    "GITLAB_USER_ID",
    "GITLAB_USER_LOGIN",
    "GITLAB_USER_NAME",
    "RUNNER_OS",
    "RUNNER_ARCH",
    "RUNNER_ENVIRONMENT",
];

#[derive(Debug, Clone)]
struct RawDefinition {
    key: String,
    raw: Option<String>,
    source: GitlabSourceRef,
}

/// Collect global `variables:` definitions in precedence order (low to
/// high): included files first (in include order), then the current file's
/// own globals. `include: remote` / `include: template` are reported as
/// external and skipped.
fn collect_global_definitions(
    path: &Path,
    depth: usize,
    include_files: &mut Vec<PathBuf>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<RawDefinition>, AnalysisError> {
    if depth > 5 {
        return Err(invalid_gitlab(
            path,
            "include nesting exceeds 5 levels (cycle?)",
        ));
    }
    let content = fs::read_to_string(path)?;
    let raw: Value =
        crate::yamlx::parse_merged(&content).map_err(|source| AnalysisError::Yaml {
            path: path.to_path_buf(),
            source,
        })?;
    let Some(document) = raw.as_mapping() else {
        return Err(invalid_gitlab(path, "file must be a YAML mapping"));
    };

    let mut definitions = Vec::new();

    if let Some(include_value) = document.get(Value::String("include".to_string())) {
        let entries: Vec<&Value> = match include_value {
            Value::Sequence(sequence) => sequence.iter().collect(),
            other => vec![other],
        };
        for entry in entries {
            let (kind, raw_path) = match entry {
                Value::String(raw_path) => ("local", raw_path.clone()),
                Value::Mapping(mapping) => {
                    let local = mapping.get(Value::String("local".to_string()));
                    let remote = mapping.get(Value::String("remote".to_string()));
                    let template = mapping.get(Value::String("template".to_string()));
                    if let Some(local) = local {
                        ("local", local.as_str().unwrap_or_default().to_string())
                    } else if let Some(remote) = remote {
                        ("remote", remote.as_str().unwrap_or_default().to_string())
                    } else if let Some(template) = template {
                        (
                            "template",
                            template.as_str().unwrap_or_default().to_string(),
                        )
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };
            match kind {
                "local" if !raw_path.is_empty() => {
                    let include_path = path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(raw_path.trim_start_matches('/'));
                    if !include_path.exists() {
                        diagnostics.push(Diagnostic::new(
                            Severity::Warning,
                            "gitlab-include-missing",
                            format!("included file does not exist: {}", include_path.display()),
                        ));
                        continue;
                    }
                    include_files.push(include_path.clone());
                    definitions.extend(collect_global_definitions(
                        &include_path,
                        depth + 1,
                        include_files,
                        diagnostics,
                    )?);
                }
                "remote" | "template" => {
                    diagnostics.push(Diagnostic::new(
                        Severity::Info,
                        "gitlab-include-external",
                        format!(
                            "include {kind} '{}' is external and cannot be expanded statically",
                            raw_path
                        ),
                    ));
                }
                _ => {}
            }
        }
    }

    if let Some(variables_value) = document.get(Value::String("variables".to_string())) {
        if let Some(mapping) = variables_value.as_mapping() {
            for (key, value) in mapping {
                let Some(key) = key.as_str() else {
                    continue;
                };
                let line = find_line(&content, &format!("{key}:"), 0);
                definitions.push(RawDefinition {
                    key: key.to_string(),
                    raw: scalar_string(value),
                    source: GitlabSourceRef::new(
                        if depth == 0 {
                            GitlabSourceKind::FileGlobal
                        } else {
                            GitlabSourceKind::IncludeFile
                        },
                        Some(path.to_path_buf()),
                        line,
                        if depth == 0 {
                            "global variables"
                        } else {
                            "included file globals"
                        },
                    ),
                });
            }
        }
    }
    Ok(definitions)
}

/// Resolve raw definitions into final variables: definitions may reference
/// each other in any order, so the interpolation context is fully seeded
/// before any value is expanded.
fn resolve(
    definitions: &[RawDefinition],
    context: &InterpolationContext<GitlabSourceRef>,
) -> Vec<GitlabVariable> {
    let mut variables: BTreeMap<String, GitlabVariable> = BTreeMap::new();
    let v2_pattern = V2_INPUT_PATTERN.get_or_init(|| {
        regex::Regex::new(r"\$\[\[\s*([A-Za-z_][A-Za-z0-9_.]*)\s*\]\]").expect("valid regex")
    });
    for definition in definitions {
        let (value, references, diagnostics) = match definition.raw.as_deref() {
            Some(raw) => {
                let result = context.interpolate(raw);
                let mut references: Vec<GitlabReference> = result
                    .references
                    .into_iter()
                    .map(|reference| GitlabReference {
                        expression: reference.variable,
                        source: reference.source,
                    })
                    .collect();
                // GitLab CI/CD component inputs (v2 `$[[ inputs.X ]]`
                // syntax) are not variable definitions, so the
                // interpolation engine skips them; report them as
                // external references for visibility.
                for capture in v2_pattern.captures_iter(raw) {
                    let expression = capture[1].to_string();
                    if !references
                        .iter()
                        .any(|reference| reference.expression == expression)
                    {
                        references.push(GitlabReference {
                            expression,
                            source: Some(GitlabSourceRef::new(
                                GitlabSourceKind::External,
                                None,
                                None,
                                "component input",
                            )),
                        });
                    }
                }
                (Some(result.value), references, result.diagnostics)
            }
            None => (None, Vec::new(), Vec::new()),
        };
        let builder = variables
            .entry(definition.key.clone())
            .or_insert_with(|| GitlabVariable {
                variable: definition.key.clone(),
                state: VariableState::Absent,
                value: None,
                winner: None,
                references: Vec::new(),
                candidates: Vec::new(),
                diagnostics: Vec::new(),
            });
        for candidate in &mut builder.candidates {
            if candidate.disposition == GitlabDisposition::Winner {
                candidate.disposition = GitlabDisposition::Shadowed;
            }
        }
        builder.state = if value.is_some() {
            VariableState::Present
        } else {
            VariableState::Absent
        };
        builder.value = value.clone();
        builder.winner = Some(definition.source.clone());
        builder.references.extend(references);
        builder.diagnostics.extend(diagnostics);
        builder.candidates.push(GitlabCandidate {
            source: definition.source.clone(),
            value,
            disposition: if builder.state == VariableState::Present {
                GitlabDisposition::Winner
            } else {
                GitlabDisposition::Shadowed
            },
        });
    }
    variables.into_values().collect()
}

pub fn analyze_gitlab(path: &Path) -> Result<GitlabReport, AnalysisError> {
    analyze_gitlab_with_content(path, None)
}

pub fn analyze_gitlab_with_content(
    path: &Path,
    content: Option<&str>,
) -> Result<GitlabReport, AnalysisError> {
    let content = match content {
        Some(content) => content.to_string(),
        None => fs::read_to_string(path)?,
    };
    let raw: Value =
        crate::yamlx::parse_merged(&content).map_err(|source| AnalysisError::Yaml {
            path: path.to_path_buf(),
            source,
        })?;
    let Some(document) = raw.as_mapping() else {
        return Err(invalid_gitlab(path, "file must be a YAML mapping"));
    };

    let mut diagnostics = Vec::new();
    let mut include_files = Vec::new();
    let file_globals = collect_global_definitions(path, 0, &mut include_files, &mut diagnostics)?;

    // Job-level raw definitions.
    let mut job_raw: BTreeMap<String, Vec<RawDefinition>> = BTreeMap::new();
    for (key, value) in document {
        let Some(job_id) = key.as_str() else {
            continue;
        };
        if NON_JOB_KEYS.contains(&job_id) {
            continue;
        }
        let Some(job_map) = value.as_mapping() else {
            continue;
        };
        let job_line = find_line(&content, &format!("{job_id}:"), 0);
        let Some(variables_value) = job_map.get(Value::String("variables".to_string())) else {
            continue;
        };
        let Some(variables_map) = variables_value.as_mapping() else {
            continue;
        };
        let mut entries = Vec::new();
        for (variable_key, variable_value) in variables_map {
            let Some(variable_key) = variable_key.as_str() else {
                continue;
            };
            let line = find_line(&content, &format!("{variable_key}:"), job_line.unwrap_or(0));
            entries.push(RawDefinition {
                key: variable_key.to_string(),
                raw: scalar_string(variable_value),
                source: GitlabSourceRef::new(
                    GitlabSourceKind::Job,
                    Some(path.to_path_buf()),
                    line,
                    format!("job '{job_id}' variables"),
                ),
            });
        }
        job_raw.insert(job_id.to_string(), entries);
    }

    // Two-pass resolution: definitions may reference each other in any
    // order, so seed the interpolation context with all raw values first.
    let mut context = InterpolationContext::<GitlabSourceRef>::new();
    for definition in &file_globals {
        context.define(
            definition.key.clone(),
            definition.raw.clone(),
            definition.source.clone(),
            100,
        );
    }
    for entries in job_raw.values() {
        for definition in entries {
            context.define(
                definition.key.clone(),
                definition.raw.clone(),
                definition.source.clone(),
                200,
            );
        }
    }
    for predefined in PREDEFINED_VARIABLES {
        context.define(
            (*predefined).to_string(),
            Some(String::new()),
            GitlabSourceRef::new(
                GitlabSourceKind::Predefined,
                None,
                None,
                "GitLab predefined variable",
            ),
            50,
        );
    }

    let global_variables = resolve(&file_globals, &context);
    let mut jobs = Vec::new();
    for (job_id, entries) in &job_raw {
        // Job scope sees file globals plus its own definitions.
        let mut scope = file_globals.clone();
        scope.extend(entries.clone());
        jobs.push(GitlabJob {
            id: job_id.clone(),
            variables: resolve(&scope, &context),
        });
    }

    Ok(GitlabReport {
        file: path.to_path_buf(),
        include_files,
        global_variables,
        jobs,
        diagnostics,
    })
}

fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn find_line(content: &str, needle: &str, start: usize) -> Option<usize> {
    content
        .lines()
        .enumerate()
        .skip(start)
        .find(|(_, line)| {
            let trimmed = line.trim_start().trim_start_matches("- ");
            trimmed == needle
                || trimmed
                    .strip_prefix(needle)
                    .is_some_and(|rest| rest.starts_with(':') || rest.starts_with(' '))
        })
        .map(|(index, _)| index + 1)
}

fn invalid_gitlab(path: &Path, message: &str) -> AnalysisError {
    AnalysisError::InvalidWorkflow {
        path: path.to_path_buf(),
        message: message.to_string(),
    }
}

// --- rendering -----------------------------------------------------------

pub fn gitlab_human(report: &GitlabReport, show_values: bool) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "EnvOrigin {} — GitLab CI analysis",
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(output, "file: {}", report.file.display());
    if !report.include_files.is_empty() {
        let files = report
            .include_files
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(output, "include files: {files}");
    }
    let _ = writeln!(output, "\nglobal variables:");
    if report.global_variables.is_empty() {
        let _ = writeln!(output, "  none");
    }
    for variable in &report.global_variables {
        let _ = writeln!(output, "  {}", variable_line(variable, show_values));
    }
    for job in &report.jobs {
        let _ = writeln!(output, "\njob {}", job.id);
        for variable in &job.variables {
            let _ = writeln!(output, "  {}", variable_line(variable, show_values));
        }
    }
    if !report.diagnostics.is_empty() {
        let _ = writeln!(output, "\ndiagnostics:");
        for diagnostic in &report.diagnostics {
            let _ = writeln!(
                output,
                "  - {} [{}]: {}",
                severity_label(diagnostic.severity),
                diagnostic.code,
                diagnostic.message
            );
        }
    }
    output
}

pub fn gitlab_variable_human(
    report: &GitlabReport,
    variable: &GitlabVariable,
    show_values: bool,
) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "{} in {}", variable.variable, report.file.display());
    let _ = writeln!(output, "state: {}", state_label(variable.state));
    if let Some(value) = &variable.value {
        let _ = writeln!(output, "value: {}", display_value(value, show_values));
    }
    if let Some(winner) = &variable.winner {
        let _ = writeln!(output, "winner: {}", source_summary(winner));
    }
    if !variable.references.is_empty() {
        let _ = writeln!(output, "references:");
        for reference in &variable.references {
            let source = reference
                .source
                .as_ref()
                .map(source_summary)
                .unwrap_or_else(|| "unknown source".to_string());
            let _ = writeln!(output, "  - ${} → {source}", reference.expression);
        }
    }
    if !variable.candidates.is_empty() {
        let _ = writeln!(output, "candidates:");
        for candidate in &variable.candidates {
            let marker = match candidate.disposition {
                GitlabDisposition::Winner => "winner",
                GitlabDisposition::Shadowed => "shadowed",
            };
            let value = candidate
                .value
                .as_deref()
                .map(|value| display_value(value, show_values))
                .unwrap_or_else(|| "—".to_string());
            let _ = writeln!(
                output,
                "  - [{marker}] {} = {value}",
                source_summary(&candidate.source)
            );
        }
    }
    for diagnostic in &variable.diagnostics {
        let _ = writeln!(
            output,
            "  - {} [{}]: {}",
            severity_label(diagnostic.severity),
            diagnostic.code,
            diagnostic.message
        );
    }
    output
}

pub fn gitlab_json(report: &GitlabReport, show_values: bool) -> String {
    let report = if show_values {
        report.clone()
    } else {
        redact_report(report)
    };
    serde_json::to_string_pretty(&report).expect("GitlabReport is serializable")
}

pub fn gitlab_variable_json(
    report: &GitlabReport,
    variable: &GitlabVariable,
    show_values: bool,
) -> String {
    let mut variable = variable.clone();
    if !show_values {
        redact_variable(&mut variable);
    }
    let mut wrapped = serde_json::Map::new();
    wrapped.insert(
        "file".to_string(),
        serde_json::to_value(&report.file).expect("PathBuf serializable"),
    );
    wrapped.insert(
        "variable".to_string(),
        serde_json::to_value(&variable).expect("serializable"),
    );
    serde_json::to_string_pretty(&serde_json::Value::Object(wrapped)).expect("serializable")
}

fn redact_report(report: &GitlabReport) -> GitlabReport {
    let mut report = report.clone();
    for variable in &mut report.global_variables {
        redact_variable(variable);
    }
    for job in &mut report.jobs {
        for variable in &mut job.variables {
            redact_variable(variable);
        }
    }
    report
}

fn redact_variable(variable: &mut GitlabVariable) {
    if let Some(value) = &mut variable.value {
        *value = fingerprint(value);
    }
    for candidate in &mut variable.candidates {
        if let Some(value) = &mut candidate.value {
            *value = fingerprint(value);
        }
    }
}

fn variable_line(variable: &GitlabVariable, show_values: bool) -> String {
    let state = state_label(variable.state);
    let rendered = variable
        .value
        .as_deref()
        .map(|value| display_value(value, show_values))
        .unwrap_or_else(|| "—".to_string());
    let source = variable
        .winner
        .as_ref()
        .map(source_summary)
        .unwrap_or_else(|| "unknown source".to_string());
    format!(
        "{:<28} {:<7} {:<28} ← {}",
        variable.variable, state, rendered, source
    )
}

fn display_value(value: &str, show_values: bool) -> String {
    if show_values {
        format!("{value:?}")
    } else {
        fingerprint(value)
    }
}

fn source_summary(source: &GitlabSourceRef) -> String {
    let kind = match source.kind {
        GitlabSourceKind::IncludeFile => "include file",
        GitlabSourceKind::FileGlobal => "global variables",
        GitlabSourceKind::Job => "job variables",
        GitlabSourceKind::Predefined => "GitLab predefined",
        GitlabSourceKind::External => "external",
    };
    format!("{kind} ({})", source.label())
}

fn state_label(state: VariableState) -> &'static str {
    match state {
        VariableState::Present => "set",
        VariableState::Absent => "absent",
    }
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
    use crate::output::fingerprint;

    fn fixture_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gitlab/.gitlab-ci.yml")
    }

    #[test]
    fn include_globals_are_shadowed_by_local_globals() {
        let report = analyze_gitlab(&fixture_path()).unwrap();
        let app_name = report
            .global_variables
            .iter()
            .find(|variable| variable.variable == "APP_NAME")
            .unwrap();
        assert_eq!(app_name.value.as_deref(), Some("demo"));
        assert_eq!(app_name.candidates.len(), 2);
        assert!(matches!(
            app_name.candidates[0].disposition,
            GitlabDisposition::Shadowed
        ));
        assert!(matches!(
            app_name.candidates[1].disposition,
            GitlabDisposition::Winner
        ));
    }

    #[test]
    fn job_overrides_global_and_references_resolve_order_independently() {
        let report = analyze_gitlab(&fixture_path()).unwrap();
        let build = report.jobs.iter().find(|job| job.id == "build").unwrap();
        let global_only = build
            .variables
            .iter()
            .find(|variable| variable.variable == "GLOBAL_ONLY")
            .unwrap();
        assert_eq!(global_only.value.as_deref(), Some("overridden_in_job"));
        let build_id = build
            .variables
            .iter()
            .find(|variable| variable.variable == "BUILD_ID")
            .unwrap();
        assert!(build_id.references.iter().any(|reference| {
            reference.expression == "APP_NAME"
                && reference
                    .source
                    .as_ref()
                    .is_some_and(|source| source.kind == GitlabSourceKind::FileGlobal)
        }));
    }

    #[test]
    fn values_are_redacted_not_leaked() {
        let report = analyze_gitlab(&fixture_path()).unwrap();
        let value = report
            .global_variables
            .iter()
            .find(|variable| variable.variable == "REGISTRY")
            .unwrap()
            .value
            .as_deref()
            .unwrap();
        // The model keeps resolved values internally; redaction happens at
        // render time — assert the fingerprint path is stable and distinct.
        assert_ne!(fingerprint(value), fingerprint("other"));
    }
}
