//! GitHub Actions workflow environment analysis.
//!
//! Static analysis of where a workflow's environment variables come from.
//! A workflow cannot be executed here, so runtime-only behavior is marked
//! explicitly instead of being guessed: `GITHUB_ENV` writes produce an Info
//! diagnostic, and `${{ secrets.* }}` / `${{ vars.* }}` references are
//! reported as external (repository or organization state).

use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::model::{AnalysisError, Diagnostic, Severity, VariableState};
use crate::output::fingerprint;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionsSourceKind {
    WorkflowEnv,
    JobEnv,
    StepEnv,
    EnvFile,
    WorkflowInput,
    DefaultVariable,
    External,
    Runtime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionsSourceRef {
    pub kind: ActionsSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub detail: String,
}

impl ActionsSourceRef {
    pub fn new(
        kind: ActionsSourceKind,
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
                ActionsSourceKind::DefaultVariable => "GitHub built-in".to_string(),
                ActionsSourceKind::External => "repository/org state".to_string(),
                ActionsSourceKind::WorkflowInput => "workflow input".to_string(),
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
pub enum ActionsDisposition {
    Winner,
    Shadowed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionsCandidate {
    pub source: ActionsSourceRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub disposition: ActionsDisposition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionsReference {
    /// Expression body, e.g. `env.SHARED`.
    pub expression: String,
    /// First context segment: env, secrets, vars, github, matrix, ...
    pub context: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ActionsSourceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionsVariable {
    pub variable: String,
    pub state: VariableState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<ActionsSourceRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ActionsReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<ActionsCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionsInput {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionsStep {
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uses: Option<String>,
    pub variables: Vec<ActionsVariable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionsJob {
    pub id: String,
    pub variables: Vec<ActionsVariable>,
    pub steps: Vec<ActionsStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionsReport {
    pub workflow_file: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<ActionsInput>,
    pub jobs: Vec<ActionsJob>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl ActionsReport {
    /// Resolve a variable by job id and optional step index.
    /// With `step_index == None` the job-level variable is returned.
    pub fn explain(
        &self,
        job: &str,
        step_index: Option<usize>,
        variable: &str,
    ) -> Result<&ActionsVariable, AnalysisError> {
        let job_report = self
            .jobs
            .iter()
            .find(|candidate| candidate.id == job)
            .ok_or_else(|| AnalysisError::UnknownJob(job.to_string()))?;
        match step_index {
            None => job_report
                .variables
                .iter()
                .find(|candidate| candidate.variable == variable)
                .ok_or_else(|| AnalysisError::UnknownActionsVariable {
                    job: job.to_string(),
                    variable: variable.to_string(),
                }),
            Some(index) => job_report
                .steps
                .iter()
                .find(|candidate| candidate.index == index)
                .ok_or_else(|| AnalysisError::UnknownStep {
                    job: job.to_string(),
                    step: index.to_string(),
                })?
                .variables
                .iter()
                .find(|candidate| candidate.variable == variable)
                .ok_or_else(|| AnalysisError::UnknownActionsVariable {
                    job: job.to_string(),
                    variable: variable.to_string(),
                }),
        }
    }
}

/// Built-in default environment variables GitHub provides to every job.
const DEFAULT_VARIABLES: &[&str] = &[
    "CI",
    "GITHUB_ACTION",
    "GITHUB_ACTION_PATH",
    "GITHUB_ACTIONS",
    "GITHUB_ACTOR",
    "GITHUB_API_URL",
    "GITHUB_BASE_REF",
    "GITHUB_ENV",
    "GITHUB_EVENT_NAME",
    "GITHUB_EVENT_PATH",
    "GITHUB_HEAD_REF",
    "GITHUB_JOB",
    "GITHUB_OUTPUT",
    "GITHUB_PATH",
    "GITHUB_REF",
    "GITHUB_REF_NAME",
    "GITHUB_REPOSITORY",
    "GITHUB_REPOSITORY_OWNER",
    "GITHUB_RUN_ID",
    "GITHUB_RUN_NUMBER",
    "GITHUB_SERVER_URL",
    "GITHUB_SHA",
    "GITHUB_STEP_SUMMARY",
    "GITHUB_WORKFLOW",
    "HOME",
    "RUNNER_ARCH",
    "RUNNER_DEBUG",
    "RUNNER_NAME",
    "RUNNER_OS",
    "RUNNER_TEMP",
    "RUNNER_TOOL_CACHE",
];

/// Contexts that cannot be resolved statically.
const EXTERNAL_CONTEXTS: &[&str] = &[
    "github", "secrets", "vars", "needs", "matrix", "steps", "strategy", "job",
];

#[derive(Debug, Clone)]
struct EnvEntry {
    key: String,
    value: Option<String>,
    line: Option<usize>,
}

/// One declaration layer: a parsed `env:` block or an env file.
#[derive(Debug, Clone)]
struct EnvLayer {
    kind: ActionsSourceKind,
    path: Option<PathBuf>,
    entries: Vec<EnvEntry>,
}

pub fn analyze_workflow(
    path: &Path,
    project_directory: Option<&Path>,
) -> Result<ActionsReport, AnalysisError> {
    analyze_workflow_with_content(path, project_directory, None)
}

pub fn analyze_workflow_with_content(
    path: &Path,
    project_directory: Option<&Path>,
    content: Option<&str>,
) -> Result<ActionsReport, AnalysisError> {
    let content = match content {
        Some(content) => content.to_string(),
        None => fs::read_to_string(path)?,
    };
    let raw: Value =
        crate::yamlx::parse_merged(&content).map_err(|source| AnalysisError::Yaml {
            path: path.to_path_buf(),
            source,
        })?;
    let document = raw
        .as_mapping()
        .ok_or_else(|| invalid_workflow(path, "workflow file must be a YAML mapping"))?;

    let name = document
        .get(Value::String("name".to_string()))
        .and_then(Value::as_str)
        .map(str::to_string);

    let inputs = parse_workflow_inputs(document, &content);

    let diagnostics = Vec::new();
    let workflow_layer = parse_env_layer(
        document.get(Value::String("env".to_string())),
        ActionsSourceKind::WorkflowEnv,
        path,
        &content,
        None,
    )?;

    let jobs_value = document
        .get(Value::String("jobs".to_string()))
        .ok_or_else(|| invalid_workflow(path, "workflow has no jobs"))?;
    let jobs_mapping = jobs_value
        .as_mapping()
        .ok_or_else(|| invalid_workflow(path, "jobs must be a mapping"))?;

    let mut jobs = Vec::new();
    for (job_key, job_value) in jobs_mapping {
        let Some(job_id) = job_key.as_str() else {
            continue;
        };
        let Some(job_map) = job_value.as_mapping() else {
            continue;
        };
        let job_start = find_line(&content, job_id, 0);
        let job_layer = parse_env_layer(
            job_map.get(Value::String("env".to_string())),
            ActionsSourceKind::JobEnv,
            path,
            &content,
            job_start,
        )?;
        let job_file_layer = parse_env_file_layer(&job_layer, path, project_directory)?;
        let job_map_layer = filter_layer(&job_layer, |key| key != "file");

        let job_variables = build_variables(
            &[
                Some(&workflow_layer),
                Some(&job_map_layer),
                job_file_layer.as_ref(),
            ],
            path,
            &inputs,
        );

        let mut steps = Vec::new();
        if let Some(steps_value) = job_map.get(Value::String("steps".to_string())) {
            if let Some(step_sequence) = steps_value.as_sequence() {
                for (index, step_value) in step_sequence.iter().enumerate() {
                    let Some(step_map) = step_value.as_mapping() else {
                        continue;
                    };
                    let step_name = step_map
                        .get(Value::String("name".to_string()))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let step_id = step_map
                        .get(Value::String("id".to_string()))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let step_uses = step_map
                        .get(Value::String("uses".to_string()))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let step_run = step_map
                        .get(Value::String("run".to_string()))
                        .and_then(Value::as_str)
                        .map(str::to_string);

                    let step_start = step_name
                        .as_deref()
                        .and_then(|name| {
                            find_line(&content, &format!("name: {name}"), job_start.unwrap_or(0))
                        })
                        .or_else(|| {
                            step_uses.as_deref().and_then(|uses| {
                                find_line(
                                    &content,
                                    &format!("uses: {uses}"),
                                    job_start.unwrap_or(0),
                                )
                            })
                        })
                        .or_else(|| {
                            step_run.as_deref().and_then(|run| {
                                find_line(&content, &format!("run: {run}"), job_start.unwrap_or(0))
                            })
                        });

                    let step_layer = parse_env_layer(
                        step_map.get(Value::String("env".to_string())),
                        ActionsSourceKind::StepEnv,
                        path,
                        &content,
                        step_start,
                    )?;
                    let step_file_layer =
                        parse_env_file_layer(&step_layer, path, project_directory)?;
                    let step_map_layer = filter_layer(&step_layer, |key| key != "file");

                    let mut step_diagnostics = Vec::new();
                    if let Some(run) = &step_run {
                        collect_runtime_writes(run, &mut step_diagnostics);
                    }

                    let step_variables = build_variables(
                        &[
                            Some(&workflow_layer),
                            Some(&job_map_layer),
                            job_file_layer.as_ref(),
                            Some(&step_map_layer),
                            step_file_layer.as_ref(),
                        ],
                        path,
                        &inputs,
                    );

                    steps.push(ActionsStep {
                        index,
                        id: step_id,
                        name: step_name,
                        uses: step_uses,
                        variables: step_variables,
                        diagnostics: step_diagnostics,
                    });
                }
            }
        }

        jobs.push(ActionsJob {
            id: job_id.to_string(),
            variables: job_variables,
            steps,
        });
    }

    if jobs.is_empty() {
        return Err(invalid_workflow(path, "workflow defines no jobs"));
    }

    Ok(ActionsReport {
        workflow_file: path.to_path_buf(),
        name,
        inputs,
        jobs,
        diagnostics,
    })
}

/// Parse `on: workflow_dispatch` / `on: workflow_call` input declarations.
fn parse_workflow_inputs(document: &serde_yaml::Mapping, content: &str) -> Vec<ActionsInput> {
    let Some(on_value) = document.get(Value::String("on".to_string())) else {
        return Vec::new();
    };
    let Value::Mapping(on_map) = on_value else {
        return Vec::new();
    };
    let mut inputs = Vec::new();
    for trigger in ["workflow_dispatch", "workflow_call"] {
        let Some(trigger_value) = on_map.get(Value::String(trigger.to_string())) else {
            continue;
        };
        let Some(trigger_map) = trigger_value.as_mapping() else {
            continue;
        };
        let Some(inputs_value) = trigger_map.get(Value::String("inputs".to_string())) else {
            continue;
        };
        let Some(inputs_map) = inputs_value.as_mapping() else {
            continue;
        };
        let trigger_line = find_line(content, &format!("{trigger}:"), 0);
        for (key, spec) in inputs_map {
            let Some(name) = key.as_str() else {
                continue;
            };
            let (description, default, required) = match spec.as_mapping() {
                Some(mapping) => (
                    mapping
                        .get(Value::String("description".to_string()))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    mapping
                        .get(Value::String("default".to_string()))
                        .and_then(scalar_string),
                    mapping
                        .get(Value::String("required".to_string()))
                        .and_then(Value::as_bool),
                ),
                None => (None, None, None),
            };
            inputs.push(ActionsInput {
                name: name.to_string(),
                description,
                default,
                required,
                line: find_line(content, &format!("{name}:"), trigger_line.unwrap_or(0)),
            });
        }
    }
    inputs
}
/// kept as an ordinary entry so `parse_env_file_layer` can find it.
fn parse_env_layer(
    value: Option<&Value>,
    kind: ActionsSourceKind,
    workflow_path: &Path,
    content: &str,
    start_line: Option<usize>,
) -> Result<EnvLayer, AnalysisError> {
    let Some(value) = value else {
        return Ok(EnvLayer {
            kind,
            path: None,
            entries: Vec::new(),
        });
    };
    let Some(mapping) = value.as_mapping() else {
        return Err(invalid_workflow(
            workflow_path,
            &format!("{kind:?} env must be a mapping"),
        ));
    };
    let mut entries = Vec::new();
    for (key, entry_value) in mapping {
        let Some(key) = key.as_str() else {
            continue;
        };
        entries.push(EnvEntry {
            key: key.to_string(),
            value: scalar_string(entry_value),
            line: find_line(content, &format!("{key}:"), start_line.unwrap_or(0)),
        });
    }
    Ok(EnvLayer {
        kind,
        path: None,
        entries,
    })
}

/// Resolve the `file:` key of an env layer into a dotenv-style file layer.
/// Returns `None` when the layer has no `file:` key.
fn parse_env_file_layer(
    layer: &EnvLayer,
    workflow_path: &Path,
    project_directory: Option<&Path>,
) -> Result<Option<EnvLayer>, AnalysisError> {
    let Some(file_entry) = layer.entries.iter().find(|entry| entry.key == "file") else {
        return Ok(None);
    };
    let Some(raw_path) = file_entry.value.as_deref() else {
        return Ok(None);
    };
    // GitHub resolves env files relative to the repository root
    // (GITHUB_WORKSPACE), which is the workflow directory's parent. Prefer an
    // explicit --project-directory, then the repo root, then the workflow dir.
    // Repository root for the standard `.github/workflows/` layout; env files
    // may also sit next to the workflow file itself.
    let repo_root = workflow_path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent);
    let mut candidates = project_directory
        .map(|dir| dir.join(raw_path))
        .into_iter()
        .chain(repo_root.map(|root| root.join(raw_path)))
        .chain(workflow_path.parent().map(|dir| dir.join(raw_path)));
    let Some(path) = candidates.find(|candidate| candidate.exists()) else {
        return Err(invalid_workflow(
            workflow_path,
            &format!(
                "env file '{raw_path}' (line {}) not found next to the workflow or at the repository root",
                file_entry
                    .line
                    .map(|line| line.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            ),
        ));
    };
    let content = fs::read_to_string(&path)?;
    let mut entries = Vec::new();
    for (offset, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some((key, value)) => (key.trim(), Some(value.trim().to_string())),
            None => (line, None),
        };
        entries.push(EnvEntry {
            key: key.to_string(),
            value,
            line: Some(offset + 1),
        });
    }
    Ok(Some(EnvLayer {
        kind: ActionsSourceKind::EnvFile,
        path: Some(path),
        entries,
    }))
}

fn filter_layer(layer: &EnvLayer, keep: impl Fn(&str) -> bool) -> EnvLayer {
    EnvLayer {
        kind: layer.kind.clone(),
        path: layer.path.clone(),
        entries: layer
            .entries
            .iter()
            .filter(|entry| keep(&entry.key))
            .cloned()
            .collect(),
    }
}

/// Build the variable set for one scope by applying the visible layers in
/// GitHub's precedence order (low → high): workflow env, job env, job env
/// file, step env, step env file.
fn build_variables(
    layers: &[Option<&EnvLayer>],
    workflow_path: &Path,
    inputs: &[ActionsInput],
) -> Vec<ActionsVariable> {
    let mut variables: BTreeMap<String, ActionsVariable> = BTreeMap::new();
    for layer in layers.iter().flatten() {
        for entry in &layer.entries {
            let source = ActionsSourceRef::new(
                layer.kind.clone(),
                layer
                    .path
                    .clone()
                    .or_else(|| Some(workflow_path.to_path_buf())),
                entry.line,
                match layer.kind {
                    ActionsSourceKind::WorkflowEnv => "workflow env",
                    ActionsSourceKind::JobEnv => "job env",
                    ActionsSourceKind::StepEnv => "step env",
                    ActionsSourceKind::EnvFile => "env file",
                    _ => "declaration",
                },
            );
            apply(&mut variables, entry, source, workflow_path, inputs);
        }
    }
    variables.into_values().collect()
}

#[allow(clippy::too_many_arguments)]
fn apply(
    variables: &mut BTreeMap<String, ActionsVariable>,
    entry: &EnvEntry,
    source: ActionsSourceRef,
    workflow_path: &Path,
    inputs: &[ActionsInput],
) {
    let references = entry
        .value
        .as_deref()
        .map(|raw| references_in(raw, variables, workflow_path, inputs))
        .unwrap_or_default();
    let builder = variables
        .entry(entry.key.clone())
        .or_insert_with(|| ActionsVariable {
            variable: entry.key.clone(),
            state: VariableState::Absent,
            value: None,
            winner: None,
            references: Vec::new(),
            candidates: Vec::new(),
            diagnostics: Vec::new(),
        });
    for candidate in &mut builder.candidates {
        if candidate.disposition == ActionsDisposition::Winner {
            candidate.disposition = ActionsDisposition::Shadowed;
        }
    }
    builder.state = if entry.value.is_some() {
        VariableState::Present
    } else {
        VariableState::Absent
    };
    builder.value = entry.value.clone();
    builder.winner = Some(source.clone());
    builder.references.extend(references);
    builder.candidates.push(ActionsCandidate {
        source,
        value: entry.value.clone(),
        disposition: if builder.state == VariableState::Present {
            ActionsDisposition::Winner
        } else {
            ActionsDisposition::Shadowed
        },
    });
}

/// Extract `${{ ... }}` expressions from a value and resolve `env.X` refs
/// against the variables declared so far.
fn references_in(
    raw: &str,
    variables: &BTreeMap<String, ActionsVariable>,
    workflow_path: &Path,
    inputs: &[ActionsInput],
) -> Vec<ActionsReference> {
    let mut references = Vec::new();
    let mut rest = raw;
    while let Some(start) = rest.find("${{") {
        let after = &rest[start + 3..];
        let Some(end) = after.find("}}") else {
            break;
        };
        let expression = after[..end].trim().to_string();
        let context = expression.split('.').next().unwrap_or_default().to_string();
        let source = if context == "env" {
            let name = expression.strip_prefix("env.").map(str::trim);
            name.and_then(|name| variables.get(name))
                .and_then(|variable| variable.winner.clone())
                .or_else(|| {
                    name.filter(|name| DEFAULT_VARIABLES.contains(name))
                        .map(|_| {
                            ActionsSourceRef::new(
                                ActionsSourceKind::DefaultVariable,
                                None,
                                None,
                                "GitHub built-in default",
                            )
                        })
                })
        } else if context == "inputs" {
            let name = expression.strip_prefix("inputs.").map(str::trim);
            name.and_then(|name| inputs.iter().find(|input| input.name == name))
                .map(|input| {
                    ActionsSourceRef::new(
                        ActionsSourceKind::WorkflowInput,
                        Some(workflow_path.to_path_buf()),
                        input.line,
                        "workflow input",
                    )
                })
        } else if EXTERNAL_CONTEXTS.contains(&context.as_str()) {
            Some(ActionsSourceRef::new(
                ActionsSourceKind::External,
                None,
                None,
                format!("{context} context (repository/org state)"),
            ))
        } else {
            None
        };
        references.push(ActionsReference {
            expression,
            context,
            source,
        });
        rest = &after[end + 2..];
    }
    references
}

/// Detect `echo "VAR=..." >> "$GITHUB_ENV"` style writes in a run script.
fn collect_runtime_writes(run: &str, diagnostics: &mut Vec<Diagnostic>) {
    let pattern = Regex::new(
        r#"echo\s+(?:['"])?([A-Za-z_][A-Za-z0-9_.-]*)=[^\s>]*['"]?\s*>>\s*["$]*\{?GITHUB_ENV\}?"#,
    )
    .expect("valid regex");
    for capture in pattern.captures_iter(run) {
        if let Some(name) = capture.get(1) {
            diagnostics.push(Diagnostic::new(
                Severity::Info,
                "github-env-runtime",
                format!(
                    "{} is written to GITHUB_ENV at runtime; its value cannot be determined statically",
                    name.as_str()
                ),
            ));
        }
    }
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

/// Find the 1-based line number of the first line whose trimmed content
/// starts with `needle` followed by `:` or whitespace (or equals it),
/// searching from `start` (0-based, inclusive). List-item prefixes (`- `)
/// are stripped so `name: Set up` matches `- name: Set up`.
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

fn invalid_workflow(path: &Path, message: &str) -> AnalysisError {
    AnalysisError::InvalidWorkflow {
        path: path.to_path_buf(),
        message: message.to_string(),
    }
}

// --- rendering -----------------------------------------------------------

pub fn actions_human(report: &ActionsReport, show_values: bool) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "EnvOrigin {} — GitHub Actions analysis",
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(output, "workflow: {}", report.workflow_file.display());
    if let Some(name) = &report.name {
        let _ = writeln!(output, "name: {name}");
    }
    if !report.inputs.is_empty() {
        let _ = writeln!(output, "workflow inputs:");
        for input in &report.inputs {
            let required = input
                .required
                .map(|required| if required { "required" } else { "optional" })
                .unwrap_or("optional");
            let default = input
                .default
                .as_deref()
                .map(|default| format!(", default {default:?}"))
                .unwrap_or_default();
            let _ = writeln!(output, "  {:<24} {required}{default}", input.name);
        }
    }
    for job in &report.jobs {
        let _ = writeln!(output, "\njob {}", job.id);
        let _ = writeln!(output, "  job-level variables:");
        if job.variables.is_empty() {
            let _ = writeln!(output, "    none");
        }
        for variable in &job.variables {
            let _ = writeln!(output, "    {}", variable_line(variable, show_values));
        }
        for step in &job.steps {
            let _ = writeln!(output, "  step {}:", step_label(step));
            if step.variables.is_empty() {
                let _ = writeln!(output, "    no step-level variables");
            }
            for variable in &step.variables {
                let _ = writeln!(output, "    {}", variable_line(variable, show_values));
            }
            for diagnostic in &step.diagnostics {
                let _ = writeln!(
                    output,
                    "    note [{}]: {}",
                    diagnostic.code, diagnostic.message
                );
            }
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

pub fn actions_variable_human(
    report: &ActionsReport,
    variable: &ActionsVariable,
    show_values: bool,
) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "{} in workflow {}",
        variable.variable,
        report.workflow_file.display()
    );
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
            let _ = writeln!(
                output,
                "  - ${{ {e} }} → {source}",
                e = reference.expression
            );
        }
    }
    if !variable.candidates.is_empty() {
        let _ = writeln!(output, "candidates:");
        for candidate in &variable.candidates {
            let marker = match candidate.disposition {
                ActionsDisposition::Winner => "winner",
                ActionsDisposition::Shadowed => "shadowed",
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

pub fn actions_variable_json(
    report: &ActionsReport,
    variable: &ActionsVariable,
    show_values: bool,
) -> String {
    let mut variable = variable.clone();
    if !show_values {
        redact_variable(&mut variable);
    }
    let mut wrapped = serde_json::Map::new();
    wrapped.insert(
        "workflow_file".to_string(),
        serde_json::to_value(&report.workflow_file).expect("PathBuf serializable"),
    );
    wrapped.insert(
        "variable".to_string(),
        serde_json::to_value(&variable).expect("serializable"),
    );
    serde_json::to_string_pretty(&serde_json::Value::Object(wrapped)).expect("serializable")
}

pub fn actions_json(report: &ActionsReport, show_values: bool) -> String {
    let report = if show_values {
        report.clone()
    } else {
        redact_report(report)
    };
    serde_json::to_string_pretty(&report).expect("ActionsReport is serializable")
}

fn redact_report(report: &ActionsReport) -> ActionsReport {
    let mut report = report.clone();
    for job in &mut report.jobs {
        for variable in &mut job.variables {
            redact_variable(variable);
        }
        for step in &mut job.steps {
            for variable in &mut step.variables {
                redact_variable(variable);
            }
        }
    }
    report
}

fn redact_variable(variable: &mut ActionsVariable) {
    if let Some(value) = &mut variable.value {
        if !is_expression(value) {
            *value = fingerprint(value);
        }
    }
    for candidate in &mut variable.candidates {
        if let Some(value) = &mut candidate.value {
            if !is_expression(value) {
                *value = fingerprint(value);
            }
        }
    }
}

fn is_expression(value: &str) -> bool {
    value.contains("${{") && value.contains("}}")
}

fn variable_line(variable: &ActionsVariable, show_values: bool) -> String {
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
        "{:<24} {:<7} {:<28} ← {}",
        variable.variable, state, rendered, source
    )
}

fn display_value(value: &str, show_values: bool) -> String {
    if is_expression(value) {
        // An expression is a code reference, not a secret: show it verbatim.
        value.to_string()
    } else if show_values {
        format!("{value:?}")
    } else {
        fingerprint(value)
    }
}

fn step_label(step: &ActionsStep) -> String {
    match (&step.name, &step.uses, &step.id) {
        (Some(name), _, _) => format!("\"{name}\""),
        (None, Some(uses), _) => uses.clone(),
        (None, None, Some(id)) => format!("#{id}"),
        // 1-based step numbers for human output (internal index is 0-based).
        (None, None, None) => format!("#{}", step.index + 1),
    }
}

/// Layer-by-layer resolution trace for `explain --debug`: every definition
/// in precedence order (lowest first — later layers win), with its layer,
/// source location, and disposition.
pub fn actions_debug_trace(variable: &ActionsVariable) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "resolution trace for {} ({}):",
        variable.variable,
        layer_order_hint()
    );
    for (index, candidate) in variable.candidates.iter().enumerate() {
        let marker = match candidate.disposition {
            ActionsDisposition::Winner => "winner",
            ActionsDisposition::Shadowed => "shadowed",
        };
        let value = candidate
            .value
            .as_deref()
            .map(|value| display_value(value, true))
            .unwrap_or_else(|| "—".to_string());
        let _ = writeln!(
            output,
            "  #{:<3} {:<9} {:<14} {value}",
            index + 1,
            marker,
            source_summary(&candidate.source)
        );
    }
    let _ = writeln!(output, "  {} definition(s)", variable.candidates.len());
    output
}

fn layer_order_hint() -> &'static str {
    "workflow env < job env < step env < env file < inputs; later wins"
}

fn source_summary(source: &ActionsSourceRef) -> String {
    let kind = match source.kind {
        ActionsSourceKind::WorkflowEnv => "workflow env",
        ActionsSourceKind::JobEnv => "job env",
        ActionsSourceKind::StepEnv => "step env",
        ActionsSourceKind::EnvFile => "env file",
        ActionsSourceKind::WorkflowInput => "workflow input",
        ActionsSourceKind::DefaultVariable => "GitHub built-in default",
        ActionsSourceKind::External => "external context",
        ActionsSourceKind::Runtime => "runtime",
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

    #[test]
    fn resolves_layered_precedence() {
        let workflow = parse_fixture("workflow.yaml");
        let job = workflow.jobs.iter().find(|job| job.id == "build").unwrap();
        let step = &job.steps[1];
        let shared = step
            .variables
            .iter()
            .find(|variable| variable.variable == "SHARED")
            .unwrap();
        assert_eq!(shared.value.as_deref(), Some("step_value"));
        assert_eq!(shared.candidates.len(), 3);
        assert!(matches!(
            shared.candidates[0].disposition,
            ActionsDisposition::Shadowed
        ));
        assert!(matches!(
            shared.candidates[2].disposition,
            ActionsDisposition::Winner
        ));
    }

    #[test]
    fn resolves_env_file_layer() {
        let workflow = parse_fixture("workflow.yaml");
        let job = workflow.jobs.iter().find(|job| job.id == "deploy").unwrap();
        let file = job
            .variables
            .iter()
            .find(|variable| variable.variable == "FILE_LEVEL")
            .unwrap();
        assert_eq!(file.value.as_deref(), Some("from_file"));
        assert!(matches!(
            file.winner.as_ref().unwrap().kind,
            ActionsSourceKind::EnvFile
        ));
        let shared = job
            .variables
            .iter()
            .find(|variable| variable.variable == "SHARED")
            .unwrap();
        assert_eq!(shared.value.as_deref(), Some("file_value"));
    }

    #[test]
    fn tracks_expression_references() {
        let workflow = parse_fixture("workflow.yaml");
        let job = workflow.jobs.iter().find(|job| job.id == "build").unwrap();
        let step = &job.steps[2];
        let matrix = step
            .variables
            .iter()
            .find(|variable| variable.variable == "MATRIX_OS")
            .unwrap();
        assert_eq!(matrix.references.len(), 1);
        assert_eq!(matrix.references[0].context, "matrix");
        assert!(matches!(
            matrix.references[0].source.as_ref().unwrap().kind,
            ActionsSourceKind::External
        ));
        let run = &job.steps[1];
        let combined = run
            .variables
            .iter()
            .find(|variable| variable.variable == "COMBINED")
            .unwrap();
        assert_eq!(combined.references[0].context, "env");
        assert_eq!(
            combined.references[0].source.as_ref().unwrap().kind,
            ActionsSourceKind::JobEnv
        );
    }

    #[test]
    fn detects_github_env_writes() {
        let workflow = parse_fixture("workflow.yaml");
        let job = workflow.jobs.iter().find(|job| job.id == "build").unwrap();
        let persist = &job.steps[3];
        assert_eq!(persist.diagnostics.len(), 1);
        assert_eq!(persist.diagnostics[0].code, "github-env-runtime");
    }

    #[test]
    fn resolves_workflow_dispatch_inputs() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/actions-inputs/.github/workflows/inputs.yaml");
        let workflow = analyze_workflow(&path, None).unwrap();
        assert_eq!(workflow.inputs.len(), 2);
        let environment = workflow
            .inputs
            .iter()
            .find(|input| input.name == "environment")
            .unwrap();
        assert_eq!(environment.required, Some(true));
        let job = workflow.jobs.iter().find(|job| job.id == "deploy").unwrap();
        let target = job
            .variables
            .iter()
            .find(|variable| variable.variable == "TARGET_ENV")
            .unwrap();
        let reference = &target.references[0];
        assert_eq!(reference.expression, "inputs.environment");
        assert_eq!(reference.context, "inputs");
        assert!(matches!(
            reference.source.as_ref().unwrap().kind,
            ActionsSourceKind::WorkflowInput
        ));
    }

    fn parse_fixture(name: &str) -> ActionsReport {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/actions/.github/workflows")
            .join(name);
        analyze_workflow(&path, None).unwrap()
    }
}
