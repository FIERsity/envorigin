//! Team convention rules from `envorigin.toml`.
//!
//! Audit checks that conventions, not just correctness: required variables
//! that must exist in the final environment, a naming prefix every
//! user-defined variable must follow, variables that are forbidden
//! outright, and per-variable value format patterns. Rules are additive to
//! the built-in checks.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use regex::Regex;
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
    /// Per-variable value format validation (regex, matched with `is_match`).
    pub patterns: BTreeMap<String, String>,
    /// Per-variable allowed-value whitelists.
    pub allowed: BTreeMap<String, Vec<String>>,
    /// Per-variable maximum value length.
    pub max_length: BTreeMap<String, usize>,
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

/// Apply rules to a set of resolved variables `(name, resolved value)`.
pub fn check_rules(variables: &[(String, Option<String>)], rules: &Rules) -> Vec<AuditIssue> {
    let mut issues = Vec::new();
    for required in &rules.required {
        if !variables.iter().any(|variable| variable.0 == *required) {
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
        for variable in variables {
            if !variable.0.as_str().starts_with(prefix.as_str()) {
                issues.push(AuditIssue::new(
                    Severity::Warning,
                    "naming-prefix",
                    format!(
                        "{} does not follow the required '{prefix}' prefix from envorigin.toml",
                        variable.0.as_str()
                    ),
                    None,
                    None,
                ));
            }
        }
    }
    for forbidden in &rules.forbidden {
        if variables
            .iter()
            .any(|variable| variable.0.as_str() == forbidden)
        {
            issues.push(AuditIssue::new(
                Severity::Error,
                "forbidden-variable",
                format!("{forbidden} is forbidden by envorigin.toml but is defined in the project"),
                None,
                None,
            ));
        }
    }
    for (variable_name, allowed) in &rules.allowed {
        if let Some((_, Some(value))) = variables
            .iter()
            .find(|variable| variable.0.as_str() == variable_name)
        {
            if !allowed.iter().any(|candidate| candidate == value) {
                issues.push(AuditIssue::new(
                    Severity::Error,
                    "disallowed-value",
                    format!("{variable_name} value is not in the allowed set from envorigin.toml"),
                    None,
                    None,
                ));
            }
        }
    }
    for (variable_name, max) in &rules.max_length {
        if let Some((_, Some(value))) = variables
            .iter()
            .find(|variable| variable.0.as_str() == variable_name)
        {
            if value.chars().count() > *max {
                issues.push(AuditIssue::new(
                    Severity::Error,
                    "value-too-long",
                    format!(
                        "{variable_name} exceeds the maximum length of {max} from envorigin.toml"
                    ),
                    None,
                    None,
                ));
            }
        }
    }
    for (variable_name, pattern) in &rules.patterns {
        let Some((_, Some(value))) = variables
            .iter()
            .find(|variable| variable.0.as_str() == variable_name)
        else {
            continue;
        };
        match Regex::new(pattern) {
            Ok(regex) if !regex.is_match(value) => issues.push(AuditIssue::new(
                Severity::Error,
                "pattern-mismatch",
                format!(
                    "{variable_name} does not match the required pattern '{pattern}' from envorigin.toml"
                ),
                None,
                None,
            )),
            Ok(_) => {}
            Err(error) => issues.push(AuditIssue::new(
                Severity::Error,
                "invalid-pattern",
                format!("invalid pattern for {variable_name} in envorigin.toml: {error}"),
                None,
                None,
            )),
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
            ..Rules::default()
        };
        let variables = vec![
            ("APP_OK".to_string(), Some("v".to_string())),
            ("BAD".to_string(), Some("v".to_string())),
        ];
        let issues = check_rules(&variables, &rules);
        assert_eq!(issues.len(), 3);
        let codes: Vec<&str> = issues.iter().map(|issue| issue.code.as_str()).collect();
        assert!(codes.contains(&"required-variable-missing"));
        assert!(codes.contains(&"naming-prefix"));
        assert!(codes.contains(&"forbidden-variable"));
    }

    #[test]
    fn pattern_mismatch_is_reported() {
        let rules = Rules {
            patterns: BTreeMap::from([(
                "DATABASE_URL".to_string(),
                "^postgres(ql)?://".to_string(),
            )]),
            ..Rules::default()
        };
        let variables = vec![("DATABASE_URL".to_string(), Some("mysql://db".to_string()))];
        let issues = check_rules(&variables, &rules);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, "pattern-mismatch");

        let ok = vec![(
            "DATABASE_URL".to_string(),
            Some("postgres://db".to_string()),
        )];
        assert!(check_rules(&ok, &rules).is_empty());
    }

    #[test]
    fn invalid_pattern_is_reported_not_panicked() {
        let rules = Rules {
            patterns: BTreeMap::from([("X".to_string(), "([".to_string())]),
            ..Rules::default()
        };
        let variables = vec![("X".to_string(), Some("value".to_string()))];
        let issues = check_rules(&variables, &rules);
        assert_eq!(issues[0].code, "invalid-pattern");
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;

    #[test]
    fn allowed_values_and_max_length_are_enforced() {
        let rules = Rules {
            allowed: BTreeMap::from([(
                "LOG_LEVEL".to_string(),
                vec!["debug".to_string(), "info".to_string()],
            )]),
            max_length: BTreeMap::from([("API_KEY".to_string(), 8)]),
            ..Rules::default()
        };
        let variables = vec![
            ("LOG_LEVEL".to_string(), Some("verbose".to_string())),
            ("API_KEY".to_string(), Some("way_too_long_key".to_string())),
        ];
        let issues = check_rules(&variables, &rules);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].code, "disallowed-value");
        assert_eq!(issues[1].code, "value-too-long");

        let ok = vec![
            ("LOG_LEVEL".to_string(), Some("info".to_string())),
            ("API_KEY".to_string(), Some("short".to_string())),
        ];
        assert!(check_rules(&ok, &rules).is_empty());
    }
}
