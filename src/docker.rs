use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::dotenv::parse_dotenv_file;
use crate::model::{Diagnostic, DockerStatus, Severity};

#[derive(Debug, Clone)]
pub struct DockerVerification {
    pub status: DockerStatus,
    pub services: BTreeMap<String, BTreeMap<String, Option<String>>>,
    pub diagnostics: Vec<Diagnostic>,
}

impl DockerVerification {
    pub fn skipped() -> Self {
        Self {
            status: DockerStatus::Skipped,
            services: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn run(
        compose_file: &Path,
        env_files: &[PathBuf],
        project_directory: &Path,
        host_env_file: Option<&Path>,
    ) -> Self {
        let mut command = Command::new("docker");
        command
            .arg("compose")
            .arg("--file")
            .arg(compose_file)
            .arg("--project-directory")
            .arg(project_directory);
        for env_file in env_files {
            command.arg("--env-file").arg(env_file);
        }
        command.arg("config").arg("--format").arg("json");

        if let Some(path) = host_env_file {
            match parse_dotenv_file(path) {
                Ok(parsed) => {
                    for entry in parsed.entries {
                        match entry.value {
                            Some(value) => {
                                command.env(entry.key, value);
                            }
                            None => {
                                command.env_remove(entry.key);
                            }
                        }
                    }
                }
                Err(error) => {
                    return Self {
                        status: DockerStatus::Failed,
                        services: BTreeMap::new(),
                        diagnostics: vec![Diagnostic::new(
                            Severity::Error,
                            "host-env-file",
                            error.to_string(),
                        )],
                    }
                }
            }
        }

        let output = match command.output() {
            Ok(output) => output,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Self {
                    status: DockerStatus::Unavailable,
                    services: BTreeMap::new(),
                    diagnostics: vec![Diagnostic::new(
                        Severity::Warning,
                        "docker-unavailable",
                        "Docker CLI was not found; analysis is unverified",
                    )],
                }
            }
            Err(error) => {
                return Self {
                    status: DockerStatus::Failed,
                    services: BTreeMap::new(),
                    diagnostics: vec![Diagnostic::new(
                        Severity::Warning,
                        "docker-command-failed",
                        error.to_string(),
                    )],
                }
            }
        };

        if !output.status.success() {
            return Self {
                status: DockerStatus::Failed,
                services: BTreeMap::new(),
                diagnostics: vec![Diagnostic::new(
                    Severity::Warning,
                    "docker-config-failed",
                    String::from_utf8_lossy(&output.stderr).trim().to_string(),
                )],
            };
        }

        match parse_services(&output.stdout) {
            Ok(services) => Self {
                status: DockerStatus::Verified,
                services,
                diagnostics: Vec::new(),
            },
            Err(message) => Self {
                status: DockerStatus::Failed,
                services: BTreeMap::new(),
                diagnostics: vec![Diagnostic::new(
                    Severity::Warning,
                    "docker-output-invalid",
                    message,
                )],
            },
        }
    }
}

fn parse_services(
    output: &[u8],
) -> Result<BTreeMap<String, BTreeMap<String, Option<String>>>, String> {
    let document: Value = serde_json::from_slice(output).map_err(|error| error.to_string())?;
    let mut result = BTreeMap::new();
    let services = document
        .get("services")
        .and_then(Value::as_object)
        .ok_or_else(|| "Docker output did not contain a services object".to_string())?;
    for (service_name, service) in services {
        let mut variables = BTreeMap::new();
        if let Some(environment) = service.get("environment").and_then(Value::as_object) {
            for (key, value) in environment {
                variables.insert(key.clone(), value.as_str().map(str::to_string));
            }
        }
        result.insert(service_name.clone(), variables);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_canonical_service_environment() {
        let json = br#"{
          "services": {
            "web": {"environment": {"A": "one", "EMPTY": ""}},
            "worker": {"image": "busybox"}
          }
        }"#;
        let parsed = parse_services(json).unwrap();
        assert_eq!(parsed["web"]["A"].as_deref(), Some("one"));
        assert_eq!(parsed["worker"].len(), 0);
    }
}
