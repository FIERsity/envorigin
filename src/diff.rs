//! Environment drift comparison between dotenv files.
//!
//! The same variable frequently ends up with different values in local `.env`,
//! CI configs, and deployment environments. `diff` makes that visible: which
//! keys drift (present in several files with different values) and which keys
//! exist in only one file. Sensitive-looking values are redacted unless
//! `--show-values` is passed.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::audit::is_sensitive;
use crate::dotenv::parse_dotenv_file;
use crate::model::AnalysisError;
use crate::output::fingerprint;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffEntry {
    pub key: String,
    /// Values in the same order as the input files.
    pub values: Vec<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffReport {
    pub files: Vec<PathBuf>,
    pub entries: Vec<DiffEntry>,
}

impl DiffReport {
    pub fn drifts(&self) -> Vec<&DiffEntry> {
        self.entries
            .iter()
            .filter(|entry| {
                let distinct = entry
                    .values
                    .iter()
                    .flatten()
                    .collect::<std::collections::HashSet<_>>()
                    .len();
                entry.values.iter().filter(|value| value.is_some()).count() > 1 && distinct > 1
            })
            .collect()
    }

    pub fn only_in(&self, index: usize) -> Vec<&DiffEntry> {
        self.entries
            .iter()
            .filter(|entry| {
                entry.values[index].is_some()
                    && entry
                        .values
                        .iter()
                        .enumerate()
                        .all(|(other, value)| other == index || value.is_none())
            })
            .collect()
    }
}

pub fn diff_files(paths: &[PathBuf]) -> Result<DiffReport, AnalysisError> {
    let mut parsed: Vec<(PathBuf, Vec<(String, Option<String>)>)> = Vec::new();
    for path in paths {
        let file = parse_dotenv_file(path)?;
        parsed.push((
            path.clone(),
            file.entries
                .into_iter()
                .map(|entry| (entry.key, entry.value))
                .collect(),
        ));
    }

    let mut keys: BTreeMap<String, Vec<Option<String>>> = BTreeMap::new();
    for (file_index, (_, entries)) in parsed.iter().enumerate() {
        for (key, value) in entries {
            let slot = keys
                .entry(key.clone())
                .or_insert_with(|| vec![None; parsed.len()]);
            slot[file_index] = value.clone();
        }
    }

    Ok(DiffReport {
        files: parsed.into_iter().map(|(path, _)| path).collect(),
        entries: keys
            .into_iter()
            .map(|(key, values)| DiffEntry { key, values })
            .collect(),
    })
}

pub fn diff_human(report: &DiffReport, show_values: bool) -> String {
    let mut output = String::new();
    use std::fmt::Write;
    let _ = writeln!(output, "EnvOrigin {} — diff", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(
        output,
        "{}",
        report
            .files
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(" vs ")
    );

    let drifts = report.drifts();
    let _ = writeln!(output, "\ndrift ({}):", drifts.len());
    for entry in drifts {
        let rendered = report
            .files
            .iter()
            .zip(&entry.values)
            .filter_map(|(path, value)| {
                value.as_ref().map(|value| {
                    format!(
                        "{} = {}",
                        path.file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.display().to_string()),
                        display_value(entry.key.as_str(), value, show_values)
                    )
                })
            })
            .collect::<Vec<_>>()
            .join(" | ");
        let _ = writeln!(output, "  {}: {}", entry.key, rendered);
    }

    for (index, path) in report.files.iter().enumerate() {
        let only = report.only_in(index);
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        let _ = writeln!(output, "\nonly in {name} ({}):", only.len());
        for entry in only {
            let value = entry.values[index]
                .as_deref()
                .map(|value| display_value(entry.key.as_str(), value, show_values))
                .unwrap_or_else(|| "—".to_string());
            let _ = writeln!(output, "  {} = {}", entry.key, value);
        }
    }
    output
}

pub fn diff_json(report: &DiffReport, show_values: bool) -> String {
    let mut report = report.clone();
    if !show_values {
        for entry in &mut report.entries {
            if is_sensitive(&entry.key) {
                for value in &mut entry.values {
                    if let Some(value) = value {
                        *value = fingerprint(value);
                    }
                }
            }
        }
    }
    serde_json::to_string_pretty(&report).expect("DiffReport is serializable")
}

fn display_value(key: &str, value: &str, show_values: bool) -> String {
    if show_values || !is_sensitive(key) {
        format!("{value:?}")
    } else {
        fingerprint(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_keys_across_three_files_without_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.env");
        let b = dir.path().join("b.env");
        let c = dir.path().join("c.env");
        std::fs::write(&a, "K=from_a\nONLY_A=1\n").unwrap();
        std::fs::write(&b, "K=from_b\nONLY_B=1\n").unwrap();
        std::fs::write(&c, "K=from_c\n").unwrap();

        let report = diff_files(&[a.clone(), b.clone(), c.clone()]).unwrap();
        let k = report
            .entries
            .iter()
            .find(|entry| entry.key == "K")
            .unwrap();
        assert_eq!(
            k.values,
            vec![
                Some("from_a".to_string()),
                Some("from_b".to_string()),
                Some("from_c".to_string())
            ]
        );
        assert_eq!(report.drifts().len(), 1);
        assert_eq!(report.only_in(0).len(), 1);
        assert_eq!(report.only_in(1).len(), 1);
        assert_eq!(report.only_in(2).len(), 0);
    }

    #[test]
    fn sensitive_values_redact_non_show_values() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.env");
        let b = dir.path().join("b.env");
        std::fs::write(&a, "TOKEN=abc\n").unwrap();
        std::fs::write(&b, "TOKEN=xyz\n").unwrap();
        let report = diff_files(&[a, b]).unwrap();
        let json = diff_json(&report, false);
        assert!(json.contains("<redacted sha256:"));
        assert!(!json.contains("\"abc\""));
        let plain = diff_json(&report, true);
        assert!(plain.contains("\"abc\""));
    }
}
