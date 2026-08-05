//! Team convention rules from `envorigin.toml`.
//!
//! Audit checks that conventions, not just correctness: required variables
//! that must exist in the final environment, a naming prefix every
//! user-defined variable must follow, and variables that are forbidden
//! outright. Rules are additive to the built-in checks.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::audit::AuditIssue;
use crate::model::Severity;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Rules {
    /// Variables that must resolve to a value somewhere in the project.
    pub required: Vec<String>,
    /// Every user-defined variable name must start with this prefix.
    pub prefix: Option<String>,
    /// Variables that must not be defined at all.
    pub forbidden: Vec<String>,
}

impl Rules {
    /// Load rules from `path`; missing files yield `None`.
    pub fn from_file(path: &Path) -> Result<Option<Rules>, String> {
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        toml::from_str(&content)
            .map(Some)
            .map_err(|error| format!("{}: {error}", path.display()))
    }
}

/// Apply rules to a set of resolved variable names (user-defined only).
pub fn check_rules(names: &[String], rules: &Rules) -> Vec<AuditIssue> {
    let mut issues = Vec::new();
    for required in &rules.required {
        if !names.iter().any(|name| name == required) {
            issues.push(AuditIssue::new(
                Severity::Error,
                "required-variable-missing",
                format!(
                    "{required} is required by envorigin.toml but does not resolve anywhere in the project"
                ),
                None,
                None,
            ));
        }
    }
    if let Some(prefix) = &rules.prefix {
        for name in names {
            if !name.starts_with(prefix.as_str()) {
                issues.push(AuditIssue::new(
                    Severity::Warning,
                    "naming-prefix",
                    format!(
                        "{name} does not follow the required '{prefix}' prefix from envorigin.toml"
                    ),
                    None,
                    None,
                ));
            }
        }
    }
    for forbidden in &rules.forbidden {
        if names.iter().any(|name| name == forbidden) {
            issues.push(AuditIssue::new(
                Severity::Error,
                "forbidden-variable",
                format!("{forbidden} is forbidden by envorigin.toml but is defined in the project"),
                None,
                None,
            ));
        }
    }
    issues
}

pub fn default_config_path() -> Option<PathBuf> {
    let path = PathBuf::from("envorigin.toml");
    path.exists().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_required_and_prefix_violations() {
        let rules = Rules {
            required: vec!["MISSING".to_string()],
            prefix: Some("APP_".to_string()),
            forbidden: vec!["BAD".to_string()],
        };
        let issues = check_rules(&["APP_OK".to_string(), "BAD".to_string()], &rules);
        assert_eq!(issues.len(), 3);
        let codes: Vec<&str> = issues.iter().map(|issue| issue.code.as_str()).collect();
        assert!(codes.contains(&"required-variable-missing"));
        assert!(codes.contains(&"naming-prefix"));
        assert!(codes.contains(&"forbidden-variable"));
    }

    #[test]
    fn prefix_violation_is_reported() {
        let rules = Rules {
            prefix: Some("APP_".to_string()),
            ..Rules::default()
        };
        let issues = check_rules(&["BAD_NAME".to_string()], &rules);
        assert_eq!(issues[0].code, "naming-prefix");
    }
}
