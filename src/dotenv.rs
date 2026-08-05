use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::model::{AnalysisError, Diagnostic, Severity};

#[derive(Debug, Clone)]
pub struct DotenvEntry {
    pub key: String,
    pub value: Option<String>,
    pub line: usize,
    pub interpolate: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DotenvFile {
    pub entries: Vec<DotenvEntry>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_dotenv_file(path: &Path) -> Result<DotenvFile, AnalysisError> {
    let content = fs::read_to_string(path)?;
    parse_dotenv(&content, path)
}

pub fn parse_dotenv(content: &str, path: &Path) -> Result<DotenvFile, AnalysisError> {
    let key_pattern = Regex::new(r"^[A-Za-z_][A-Za-z0-9_.-]*$").expect("valid regex");
    let mut result = DotenvFile::default();

    for (offset, raw_line) in content.lines().enumerate() {
        let line_number = offset + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let (key, raw_value) = match line.split_once('=') {
            Some((key, value)) => (key.trim(), Some(value.trim_start())),
            None => (line.trim(), None),
        };

        if !key_pattern.is_match(key) {
            return Err(AnalysisError::Dotenv {
                path: PathBuf::from(path),
                line: line_number,
                message: format!("invalid variable name '{key}'"),
            });
        }

        let (value, interpolate) = match raw_value {
            None => (None, false),
            Some(raw) => {
                let (parsed, allow_interpolation, trailing) =
                    parse_value(raw).map_err(|message| AnalysisError::Dotenv {
                        path: PathBuf::from(path),
                        line: line_number,
                        message,
                    })?;
                if trailing {
                    result.diagnostics.push(Diagnostic::new(
                        Severity::Warning,
                        "dotenv-trailing-content",
                        format!(
                            "{}:{line_number} has ignored trailing content",
                            path.display()
                        ),
                    ));
                }
                (Some(parsed), allow_interpolation)
            }
        };

        result.entries.push(DotenvEntry {
            key: key.to_string(),
            value,
            line: line_number,
            interpolate,
        });
    }

    Ok(result)
}

fn parse_value(raw: &str) -> Result<(String, bool, bool), String> {
    if let Some(rest) = raw.strip_prefix('\'') {
        let Some(end) = rest.find('\'') else {
            return Err("unterminated single-quoted value".to_string());
        };
        let value = rest[..end].to_string();
        let tail = rest[end + 1..].trim();
        return Ok((value, false, !tail.is_empty() && !tail.starts_with('#')));
    }

    if let Some(rest) = raw.strip_prefix('"') {
        let mut value = String::new();
        let mut escaped = false;
        let mut closing = None;
        for (index, ch) in rest.char_indices() {
            if escaped {
                value.push(match ch {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '\\' => '\\',
                    '"' => '"',
                    other => other,
                });
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                closing = Some(index);
                break;
            } else {
                value.push(ch);
            }
        }
        let Some(end) = closing else {
            return Err("unterminated double-quoted value".to_string());
        };
        let tail = rest[end + 1..].trim();
        return Ok((value, true, !tail.is_empty() && !tail.starts_with('#')));
    }

    let mut end = raw.len();
    let chars: Vec<(usize, char)> = raw.char_indices().collect();
    for window in chars.windows(2) {
        if window[0].1.is_whitespace() && window[1].1 == '#' {
            end = window[0].0;
            break;
        }
    }
    Ok((raw[..end].trim_end().to_string(), true, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quotes_comments_and_unsets() {
        let parsed = parse_dotenv(
            "A=one # comment\nB='${A}'\nC=\"line\\n${A}\"\nD\n",
            Path::new("fixture.env"),
        )
        .unwrap();

        assert_eq!(parsed.entries[0].value.as_deref(), Some("one"));
        assert_eq!(parsed.entries[1].value.as_deref(), Some("${A}"));
        assert!(!parsed.entries[1].interpolate);
        assert_eq!(parsed.entries[2].value.as_deref(), Some("line\n${A}"));
        assert!(parsed.entries[2].interpolate);
        assert_eq!(parsed.entries[3].value, None);
    }
}
