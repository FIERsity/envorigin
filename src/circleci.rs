//! CircleCI configuration environment analysis.
//!
//! Layered resolution: executor `environment:` < job `environment:` /
//! `env:` list. `<< parameters.X >>` references resolve to the job's
//! `parameters:` declaration; `<< pipeline.X >>` and `context:` are
//! organization/API state and are reported as external. `CIRCLE_*`
//! predefined variables are labelled. Values interpolate shell-style
//! (`$VAR`, `${VAR}`) through the shared interpolation engine in a second
//! pass over all definitions (order-independent, matching GitLab).

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
pub enum CircleciSourceKind {
    Executor,
    Job,
    Parameter,
    Predefined,
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CircleciSourceRef {
    pub kind: CircleciSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub detail: String,
}

impl CircleciSourceRef {
    pub fn new(
        kind: CircleciSourceKind,
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
                CircleciSourceKind::Predefined => "CircleCI predefined".to_string(),
                CircleciSourceKind::External => "external".to_string(),
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
pub enum CircleciDisposition {
    Winner,
    Shadowed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircleciCandidate {
    pub source: CircleciSourceRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub disposition: CircleciDisposition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircleciReference {
    /// Expression body, e.g. `parameters.target` or `CIRCLE_BRANCH`.
    pub expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<CircleciSourceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircleciVariable {
    pub variable: String,
    pub state: VariableState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<CircleciSourceRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<CircleciReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<CircleciCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircleciJob {
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<CircleciParameter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<String>,
    pub variables: Vec<CircleciVariable>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircleciParameter {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircleciReport {
    pub file: PathBuf,
    pub jobs: Vec<CircleciJob>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl CircleciReport {
    pub fn explain(&self, job: &str, variable: &str) -> Result<&CircleciVariable, AnalysisError> {
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
}

/// Statically-known CircleCI predefined variables (a practical subset).
const PREDEFINED_VARIABLES: &[&str] = &[
    "CIRCLE_BRANCH",
    "CIRCLE_BUILD_NUM",
    "CIRCLE_JOB",
    "CIRCLE_NODE_INDEX",
    "CIRCLE_NODE_TOTAL",
    "CIRCLE_PR_NUMBER",
    "CIRCLE_PR_REPONAME",
    "CIRCLE_PR_USERNAME",
    "CIRCLE_PROJECT_REPONAME",
    "CIRCLE_PROJECT_USERNAME",
    "CIRCLE_REPOSITORY_URL",
    "CIRCLE_SHA1",
    "CIRCLE_STAGE",
    "CIRCLE_TAG",
    "CIRCLE_USERNAME",
    "CIRCLE_WORKFLOW_ID",
    "CIRCLE_WORKFLOW_JOB_ID",
    "CIRCLE_WORKFLOW_WORKSPACE_ID",
    "CIRCLE_BRANCH_PROTECTED",
];

#[derive(Debug, Clone)]
struct RawDefinition {
    key: String,
    raw: Option<String>,
    source: CircleciSourceRef,
}

#[derive(Debug, Clone)]
struct ParameterDecl {
    name: String,
    default: Option<String>,
    line: Option<usize>,
}

fn parse_environment(
    value: Option<&Value>,
    kind: CircleciSourceKind,
    path: &Path,
    content: &str,
    start: usize,
    detail: &str,
) -> Vec<RawDefinition> {
    let mut definitions = Vec::new();
    let Some(value) = value else {
        return definitions;
    };
    match value {
        Value::Mapping(mapping) => {
            for (key, entry) in mapping {
                let Some(key) = key.as_str() else {
                    continue;
                };
                definitions.push(RawDefinition {
                    key: key.to_string(),
                    raw: scalar_string(entry),
                    source: CircleciSourceRef::new(
                        kind.clone(),
                        Some(path.to_path_buf()),
                        find_line(content, &format!("{key}:"), start),
                        detail,
                    ),
                });
            }
        }
        Value::Sequence(sequence) => {
            for item in sequence {
                let Some(item_map) = item.as_mapping() else {
                    continue;
                };
                for (key, entry) in item_map {
                    let Some(key) = key.as_str() else {
                        continue;
                    };
                    definitions.push(RawDefinition {
                        key: key.to_string(),
                        raw: scalar_string(entry),
                        source: CircleciSourceRef::new(
                            kind.clone(),
                            Some(path.to_path_buf()),
                            find_line(content, &format!("{key}:"), start),
                            detail,
                        ),
                    });
                }
            }
        }
        _ => {}
    }
    definitions
}

fn parse_parameters(value: Option<&Value>, content: &str, start: usize) -> Vec<ParameterDecl> {
    let mut parameters = Vec::new();
    let Some(value) = value else {
        return parameters;
    };
    let Some(mapping) = value.as_mapping() else {
        return parameters;
    };
    for (key, spec) in mapping {
        let Some(name) = key.as_str() else {
            continue;
        };
        let default = spec
            .as_mapping()
            .and_then(|mapping| mapping.get(Value::String("default".to_string())))
            .and_then(scalar_string);
        parameters.push(ParameterDecl {
            name: name.to_string(),
            default,
            line: find_line(content, &format!("{name}:"), start),
        });
    }
    parameters
}

pub fn analyze_circleci(path: &Path) -> Result<CircleciReport, AnalysisError> {
    analyze_circleci_with_content(path, None)
}

pub fn analyze_circleci_with_content(
    path: &Path,
    content: Option<&str>,
) -> Result<CircleciReport, AnalysisError> {
    let content = match content {
        Some(content) => content.to_string(),
        None => fs::read_to_string(path)?,
    };
    let raw: Value = serde_yaml::from_str(&content).map_err(|source| AnalysisError::Yaml {
        path: path.to_path_buf(),
        source,
    })?;
    let Some(document) = raw.as_mapping() else {
        return Err(invalid_circleci(path, "file must be a YAML mapping"));
    };

    let mut diagnostics = Vec::new();

    // Executor environments, referenced by name from jobs.
    let mut executors: BTreeMap<String, Vec<RawDefinition>> = BTreeMap::new();
    if let Some(executors_value) = document.get(Value::String("executors".to_string())) {
        if let Some(executors_map) = executors_value.as_mapping() {
            for (name, spec) in executors_map {
                let Some(name) = name.as_str() else {
                    continue;
                };
                let start = find_line(&content, &format!("{name}:"), 0).unwrap_or(0);
                let environment = spec
                    .as_mapping()
                    .and_then(|mapping| mapping.get(Value::String("environment".to_string())));
                executors.insert(
                    name.to_string(),
                    parse_environment(
                        environment,
                        CircleciSourceKind::Executor,
                        path,
                        &content,
                        start,
                        &format!("executor '{name}'"),
                    ),
                );
            }
        }
    }

    let jobs_value = document
        .get(Value::String("jobs".to_string()))
        .ok_or_else(|| invalid_circleci(path, "config has no jobs"))?;
    let jobs_map = jobs_value
        .as_mapping()
        .ok_or_else(|| invalid_circleci(path, "jobs must be a mapping"))?;

    let mut jobs = Vec::new();
    for (job_key, job_value) in jobs_map {
        let Some(job_id) = job_key.as_str() else {
            continue;
        };
        let Some(job_map) = job_value.as_mapping() else {
            continue;
        };
        let job_start = find_line(&content, &format!("{job_id}:"), 0).unwrap_or(0);

        // Executor layer.
        let mut scope: Vec<RawDefinition> = Vec::new();
        if let Some(executor_value) = job_map.get(Value::String("executor".to_string())) {
            if let Some(executor_name) = executor_value.as_str() {
                if let Some(definitions) = executors.get(executor_name) {
                    scope.extend(definitions.clone());
                } else {
                    diagnostics.push(Diagnostic::new(
                        Severity::Warning,
                        "circleci-unknown-executor",
                        format!("job '{job_id}' references unknown executor '{executor_name}'"),
                    ));
                }
            }
        }

        // Job environment layer: `environment:` map and `env:` list.
        scope.extend(parse_environment(
            job_map.get(Value::String("environment".to_string())),
            CircleciSourceKind::Job,
            path,
            &content,
            job_start,
            &format!("job '{job_id}' environment"),
        ));
        scope.extend(parse_environment(
            job_map.get(Value::String("env".to_string())),
            CircleciSourceKind::Job,
            path,
            &content,
            job_start,
            &format!("job '{job_id}' env"),
        ));

        // Contexts are org-level state: report, don't resolve.
        let mut contexts = Vec::new();
        if let Some(context_value) = job_map.get(Value::String("context".to_string())) {
            let entries: Vec<&Value> = match context_value {
                Value::Sequence(sequence) => sequence.iter().collect(),
                other => vec![other],
            };
            for entry in entries {
                if let Some(name) = entry.as_str() {
                    contexts.push(name.to_string());
                    diagnostics.push(Diagnostic::new(
                        Severity::Info,
                        "circleci-context-external",
                        format!(
                            "job '{job_id}' uses context '{name}', which is organization state and cannot be resolved statically"
                        ),
                    ));
                }
            }
        }

        let parameters = parse_parameters(
            job_map.get(Value::String("parameters".to_string())),
            &content,
            job_start,
        );

        // Two-pass resolution over the job scope (order-independent refs).
        let mut context = InterpolationContext::<CircleciSourceRef>::new();
        for definition in &scope {
            context.define(
                definition.key.clone(),
                definition.raw.clone(),
                definition.source.clone(),
                100,
            );
        }
        for parameter in &parameters {
            context.define(
                parameter.name.clone(),
                parameter.default.clone(),
                CircleciSourceRef::new(
                    CircleciSourceKind::Parameter,
                    Some(path.to_path_buf()),
                    parameter.line,
                    "job parameter",
                ),
                200,
            );
        }
        for predefined in PREDEFINED_VARIABLES {
            context.define(
                (*predefined).to_string(),
                Some(String::new()),
                CircleciSourceRef::new(
                    CircleciSourceKind::Predefined,
                    None,
                    None,
                    "CircleCI predefined variable",
                ),
                50,
            );
        }

        let mut variables: BTreeMap<String, CircleciVariable> = BTreeMap::new();
        for definition in &scope {
            let (value, references, variable_diagnostics) = match definition.raw.as_deref() {
                Some(raw) => {
                    let result = context.interpolate(raw);
                    let mut references = result
                        .references
                        .into_iter()
                        .map(|reference| CircleciReference {
                            expression: reference.variable,
                            source: reference.source,
                        })
                        .collect::<Vec<_>>();
                    references.extend(expand_refs(raw, &context));
                    (Some(result.value), references, result.diagnostics)
                }
                None => (None, Vec::new(), Vec::new()),
            };
            let builder =
                variables
                    .entry(definition.key.clone())
                    .or_insert_with(|| CircleciVariable {
                        variable: definition.key.clone(),
                        state: VariableState::Absent,
                        value: None,
                        winner: None,
                        references: Vec::new(),
                        candidates: Vec::new(),
                        diagnostics: Vec::new(),
                    });
            for candidate in &mut builder.candidates {
                if candidate.disposition == CircleciDisposition::Winner {
                    candidate.disposition = CircleciDisposition::Shadowed;
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
            builder.diagnostics.extend(variable_diagnostics);
            builder.candidates.push(CircleciCandidate {
                source: definition.source.clone(),
                value,
                disposition: if builder.state == VariableState::Present {
                    CircleciDisposition::Winner
                } else {
                    CircleciDisposition::Shadowed
                },
            });
        }

        jobs.push(CircleciJob {
            id: job_id.to_string(),
            parameters: parameters
                .into_iter()
                .map(|parameter| CircleciParameter {
                    name: parameter.name,
                    default: parameter.default,
                    line: parameter.line,
                })
                .collect(),
            context: contexts,
            variables: variables.into_values().collect(),
        });
    }

    if jobs.is_empty() {
        return Err(invalid_circleci(path, "config defines no jobs"));
    }

    Ok(CircleciReport {
        file: path.to_path_buf(),
        jobs,
        diagnostics,
    })
}

/// Track `<< parameters.X >>` / `<< pipeline.X >>` expressions, which the
/// interpolation engine does not expand.
fn expand_refs(
    raw: &str,
    context: &InterpolationContext<CircleciSourceRef>,
) -> Vec<CircleciReference> {
    let mut references = Vec::new();
    let mut rest = raw;
    while let Some(start) = rest.find("<<") {
        let after = &rest[start + 2..];
        let Some(end) = after.find(">>") else {
            break;
        };
        let expression = after[..end].trim().to_string();
        let source = if let Some(parameter) = expression.strip_prefix("parameters.") {
            context.source(parameter.trim()).cloned()
        } else if expression.starts_with("pipeline.") || expression.starts_with("context.") {
            Some(CircleciSourceRef::new(
                CircleciSourceKind::External,
                None,
                None,
                format!("{expression} (organization/API state)"),
            ))
        } else {
            None
        };
        references.push(CircleciReference { expression, source });
        rest = &after[end + 2..];
    }
    references
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

fn invalid_circleci(path: &Path, message: &str) -> AnalysisError {
    AnalysisError::InvalidWorkflow {
        path: path.to_path_buf(),
        message: message.to_string(),
    }
}

// --- rendering -----------------------------------------------------------

pub fn circleci_human(report: &CircleciReport, show_values: bool) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "EnvOrigin {} — CircleCI analysis",
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(output, "file: {}", report.file.display());
    for job in &report.jobs {
        let _ = writeln!(output, "\njob {}", job.id);
        if !job.parameters.is_empty() {
            let _ = writeln!(output, "  parameters:");
            for parameter in &job.parameters {
                let default = parameter
                    .default
                    .as_deref()
                    .map(|default| format!(" (default {default:?})"))
                    .unwrap_or_default();
                let _ = writeln!(output, "    {}{default}", parameter.name);
            }
        }
        if !job.context.is_empty() {
            let _ = writeln!(output, "  context: {}", job.context.join(", "));
        }
        if job.variables.is_empty() {
            let _ = writeln!(output, "  no environment variables");
        }
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

pub fn circleci_variable_human(
    report: &CircleciReport,
    variable: &CircleciVariable,
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
            let _ = writeln!(output, "  - {e} → {source}", e = reference.expression);
        }
    }
    if !variable.candidates.is_empty() {
        let _ = writeln!(output, "candidates:");
        for candidate in &variable.candidates {
            let marker = match candidate.disposition {
                CircleciDisposition::Winner => "winner",
                CircleciDisposition::Shadowed => "shadowed",
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

pub fn circleci_json(report: &CircleciReport, show_values: bool) -> String {
    let report = if show_values {
        report.clone()
    } else {
        redact_report(report)
    };
    serde_json::to_string_pretty(&report).expect("CircleciReport is serializable")
}

pub fn circleci_variable_json(
    report: &CircleciReport,
    variable: &CircleciVariable,
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

fn redact_report(report: &CircleciReport) -> CircleciReport {
    let mut report = report.clone();
    for job in &mut report.jobs {
        for variable in &mut job.variables {
            redact_variable(variable);
        }
    }
    report
}

fn redact_variable(variable: &mut CircleciVariable) {
    if let Some(value) = &mut variable.value {
        *value = fingerprint(value);
    }
    for candidate in &mut variable.candidates {
        if let Some(value) = &mut candidate.value {
            *value = fingerprint(value);
        }
    }
}

fn variable_line(variable: &CircleciVariable, show_values: bool) -> String {
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

fn source_summary(source: &CircleciSourceRef) -> String {
    let kind = match source.kind {
        CircleciSourceKind::Executor => "executor environment",
        CircleciSourceKind::Job => "job environment",
        CircleciSourceKind::Parameter => "job parameter",
        CircleciSourceKind::Predefined => "CircleCI predefined",
        CircleciSourceKind::External => "external",
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
