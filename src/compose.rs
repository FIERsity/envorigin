use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_yaml::Value;

use crate::dotenv::parse_dotenv_file;
use crate::interpolation::InterpolationContext;
use crate::model::{
    AnalysisError, Candidate, CandidateDisposition, Diagnostic, Explanation, ServiceReport,
    Severity, SourceKind, SourceRef, VariableState,
};

#[derive(Debug, Clone)]
pub struct ComposeFile {
    pub services: BTreeMap<String, Service>,
}

#[derive(Debug, Clone, Default)]
pub struct Service {
    pub env_files: Vec<EnvFileSpec>,
    pub environment: Vec<EnvironmentEntry>,
}

#[derive(Debug, Clone)]
pub struct EnvFileSpec {
    pub path: String,
    pub required: bool,
    pub raw: bool,
}

#[derive(Debug, Clone)]
pub struct EnvironmentEntry {
    pub key: String,
    pub value: Option<String>,
    pub line: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RawComposeFile {
    #[serde(default)]
    services: BTreeMap<String, RawService>,
}

#[derive(Debug, Deserialize, Default)]
struct RawService {
    #[serde(default)]
    env_file: Option<Value>,
    #[serde(default)]
    environment: Option<Value>,
}

#[derive(Debug, Clone)]
struct VariableBuilder {
    state: VariableState,
    value: Option<String>,
    winner: Option<SourceRef>,
    derived_from: Vec<SourceRef>,
    candidates: Vec<Candidate>,
    diagnostics: Vec<Diagnostic>,
}

impl Default for VariableBuilder {
    fn default() -> Self {
        Self {
            state: VariableState::Absent,
            value: None,
            winner: None,
            derived_from: Vec::new(),
            candidates: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

impl VariableBuilder {
    fn apply(&mut self, value: Option<String>, source: SourceRef, derived_from: Vec<SourceRef>) {
        for candidate in &mut self.candidates {
            if candidate.disposition == CandidateDisposition::Winner {
                candidate.disposition = CandidateDisposition::Shadowed;
            }
        }
        self.state = if value.is_some() {
            VariableState::Present
        } else {
            VariableState::Absent
        };
        self.value = value.clone();
        self.winner = Some(source.clone());
        self.derived_from = deduplicate_sources(derived_from);
        self.candidates.push(Candidate {
            source,
            value,
            disposition: if self.state == VariableState::Present {
                CandidateDisposition::Winner
            } else {
                CandidateDisposition::Removed
            },
        });
    }

    fn into_explanation(self, service: &str, variable: String) -> Explanation {
        Explanation {
            variable,
            service: service.to_string(),
            state: self.state,
            value: self.value,
            winner: self.winner,
            derived_from: self.derived_from,
            candidates: self.candidates,
            diagnostics: self.diagnostics,
        }
    }
}

pub fn load_compose(path: &Path) -> Result<ComposeFile, AnalysisError> {
    load_compose_with_content(path, None)
}

pub fn load_compose_with_content(
    path: &Path,
    content: Option<&str>,
) -> Result<ComposeFile, AnalysisError> {
    let content = match content {
        Some(content) => content.to_string(),
        None => fs::read_to_string(path)?,
    };
    let raw: RawComposeFile =
        serde_yaml::from_str(&content).map_err(|source| AnalysisError::Yaml {
            path: path.to_path_buf(),
            source,
        })?;
    if raw.services.is_empty() {
        return Err(AnalysisError::NoServices(path.to_path_buf()));
    }

    let mut services = BTreeMap::new();
    for (name, raw_service) in raw.services {
        services.insert(
            name.clone(),
            Service {
                env_files: normalize_env_files(raw_service.env_file.as_ref(), path)?,
                environment: normalize_environment(
                    raw_service.environment.as_ref(),
                    &content,
                    &name,
                )?,
            },
        );
    }
    Ok(ComposeFile { services })
}

pub fn analyze_service(
    name: &str,
    service: &Service,
    compose_file: &Path,
    context: &InterpolationContext<SourceRef>,
    canonical: Option<&BTreeMap<String, Option<String>>>,
) -> Result<ServiceReport, AnalysisError> {
    let compose_directory = compose_file.parent().unwrap_or_else(|| Path::new("."));
    let mut variables: BTreeMap<String, VariableBuilder> = BTreeMap::new();

    for spec in &service.env_files {
        let path_result = context.interpolate(&spec.path);
        let path = compose_directory.join(&path_result.value);
        if !path.exists() && !spec.required {
            continue;
        }
        let parsed = parse_dotenv_file(&path)?;
        for entry in parsed.entries {
            let interpolation = entry
                .value
                .as_deref()
                .map(|raw| context.interpolate_if(raw, entry.interpolate && !spec.raw));
            let derived_from = interpolation
                .as_ref()
                .map(|result| {
                    result
                        .references
                        .iter()
                        .filter_map(|reference| reference.source.clone())
                        .collect()
                })
                .unwrap_or_default();
            let builder = variables.entry(entry.key.clone()).or_default();
            if let Some(result) = &interpolation {
                builder.diagnostics.extend(result.diagnostics.clone());
            }
            builder.apply(
                interpolation.map(|result| result.value),
                SourceRef::new(
                    SourceKind::ServiceEnvFile,
                    Some(path.clone()),
                    Some(entry.line),
                    "service env_file",
                ),
                derived_from,
            );
        }
    }

    for entry in &service.environment {
        let (value, derived_from, diagnostics) = match &entry.value {
            Some(raw) => {
                let result = context.interpolate(raw);
                let sources = result
                    .references
                    .iter()
                    .filter_map(|reference| reference.source.clone())
                    .collect();
                (Some(result.value), sources, result.diagnostics)
            }
            None => match context.get(&entry.key) {
                Some(value) => (
                    Some(value.to_string()),
                    context.source(&entry.key).cloned().into_iter().collect(),
                    Vec::new(),
                ),
                None => (None, Vec::new(), Vec::new()),
            },
        };
        let builder = variables.entry(entry.key.clone()).or_default();
        builder.diagnostics.extend(diagnostics);
        builder.apply(
            value,
            SourceRef::new(
                SourceKind::ServiceEnvironment,
                Some(compose_file.to_path_buf()),
                entry.line,
                "services.*.environment",
            ),
            derived_from,
        );
    }

    if let Some(canonical) = canonical {
        let keys: BTreeSet<String> = variables.keys().chain(canonical.keys()).cloned().collect();
        for key in keys {
            let actual = canonical.get(&key).cloned().flatten();
            let builder = variables.entry(key.clone()).or_default();
            if canonical.contains_key(&key) {
                if builder.state != VariableState::Present || builder.value != actual {
                    builder.diagnostics.push(Diagnostic::new(
                        Severity::Warning,
                        "docker-divergence",
                        format!(
                            "Docker resolved {key} differently; Docker's canonical value is used"
                        ),
                    ));
                    builder.state = VariableState::Present;
                    builder.value = actual;
                    if builder.winner.is_none() {
                        let source = SourceRef::new(
                            SourceKind::DockerCanonical,
                            None,
                            None,
                            "present only in Docker's canonical Compose model",
                        );
                        builder.winner = Some(source.clone());
                        builder.candidates.push(Candidate {
                            source,
                            value: builder.value.clone(),
                            disposition: CandidateDisposition::Winner,
                        });
                    }
                }
            } else if builder.state == VariableState::Present {
                builder.diagnostics.push(Diagnostic::new(
                    Severity::Warning,
                    "docker-divergence",
                    format!("Docker removed {key} from the canonical service environment"),
                ));
                builder.state = VariableState::Absent;
                builder.value = None;
            }
        }
    }

    Ok(ServiceReport {
        name: name.to_string(),
        variables: variables
            .into_iter()
            .map(|(variable, builder)| builder.into_explanation(name, variable))
            .collect(),
    })
}

fn normalize_env_files(
    value: Option<&Value>,
    compose_path: &Path,
) -> Result<Vec<EnvFileSpec>, AnalysisError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items: Vec<&Value> = match value {
        Value::Sequence(sequence) => sequence.iter().collect(),
        other => vec![other],
    };
    let mut result = Vec::new();
    for item in items {
        match item {
            Value::String(path) => result.push(EnvFileSpec {
                path: path.clone(),
                required: true,
                raw: false,
            }),
            Value::Mapping(mapping) => {
                let path = mapping
                    .get(Value::String("path".to_string()))
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid_compose(compose_path, "env_file entry requires path"))?;
                let required = mapping
                    .get(Value::String("required".to_string()))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let raw = mapping
                    .get(Value::String("format".to_string()))
                    .and_then(Value::as_str)
                    .is_some_and(|format| format == "raw");
                result.push(EnvFileSpec {
                    path: path.to_string(),
                    required,
                    raw,
                });
            }
            _ => {
                return Err(invalid_compose(
                    compose_path,
                    "env_file must be a path or a list of path entries",
                ))
            }
        }
    }
    Ok(result)
}

fn normalize_environment(
    value: Option<&Value>,
    content: &str,
    service: &str,
) -> Result<Vec<EnvironmentEntry>, AnalysisError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    match value {
        Value::Mapping(mapping) => {
            for (key, value) in mapping {
                let Some(key) = key.as_str() else {
                    continue;
                };
                let value = yaml_scalar(value);
                result.push(EnvironmentEntry {
                    key: key.to_string(),
                    value,
                    line: find_environment_line(content, service, key),
                });
            }
        }
        Value::Sequence(sequence) => {
            for item in sequence {
                let Some(item) = item.as_str() else {
                    continue;
                };
                let (key, value) = match item.split_once('=') {
                    Some((key, value)) => (key, Some(value.to_string())),
                    None => (item, None),
                };
                result.push(EnvironmentEntry {
                    key: key.to_string(),
                    value,
                    line: find_environment_line(content, service, key),
                });
            }
        }
        _ => {
            return Err(invalid_compose(
                Path::new("compose.yaml"),
                "environment must be a map or list",
            ))
        }
    }
    Ok(result)
}

fn yaml_scalar(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => Some(value.clone()),
        _ => serde_yaml::to_string(value)
            .ok()
            .map(|value| value.trim().to_string()),
    }
}

fn find_environment_line(content: &str, service: &str, key: &str) -> Option<usize> {
    let mut in_service = false;
    let mut service_indent = 0;
    let mut in_environment = false;
    let mut environment_indent = 0;

    for (index, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if trimmed.starts_with(&format!("{service}:")) {
            in_service = true;
            service_indent = indent;
            in_environment = false;
            continue;
        }
        if in_service && !trimmed.is_empty() && indent <= service_indent {
            in_service = false;
        }
        if in_service && trimmed == "environment:" {
            in_environment = true;
            environment_indent = indent;
            continue;
        }
        if in_environment && !trimmed.is_empty() && indent <= environment_indent {
            in_environment = false;
        }
        if in_environment {
            let candidate = trimmed.trim_start_matches("- ");
            if candidate.starts_with(&format!("{key}:"))
                || candidate.starts_with(&format!("{key}="))
                || candidate == key
            {
                return Some(index + 1);
            }
        }
    }
    None
}

fn invalid_compose(path: &Path, message: &str) -> AnalysisError {
    AnalysisError::InvalidCompose {
        path: path.to_path_buf(),
        message: message.to_string(),
    }
}

fn deduplicate_sources(sources: Vec<SourceRef>) -> Vec<SourceRef> {
    let mut result = Vec::new();
    for source in sources {
        if !result
            .iter()
            .any(|existing: &SourceRef| existing == &source)
        {
            result.push(source);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_map_and_list_environment_forms() {
        let map = "services:\n  web:\n    environment:\n      A: fixed\n      B:\n";
        let parsed = load_from_str(map);
        assert_eq!(parsed.environment[0].key, "A");
        assert_eq!(parsed.environment[0].value.as_deref(), Some("fixed"));
        assert_eq!(parsed.environment[1].value, None);

        let list = "services:\n  web:\n    environment:\n      - A=fixed\n      - B\n";
        let parsed = load_from_str(list);
        assert_eq!(parsed.environment[0].value.as_deref(), Some("fixed"));
        assert_eq!(parsed.environment[1].value, None);
    }

    fn load_from_str(content: &str) -> Service {
        let raw: RawComposeFile = serde_yaml::from_str(content).unwrap();
        let raw_service = raw.services.get("web").unwrap();
        Service {
            env_files: normalize_env_files(
                raw_service.env_file.as_ref(),
                Path::new("compose.yaml"),
            )
            .unwrap(),
            environment: normalize_environment(raw_service.environment.as_ref(), content, "web")
                .unwrap(),
        }
    }
}
