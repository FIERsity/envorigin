use std::collections::HashMap;

use serde::Serialize;

use crate::model::{Diagnostic, Severity};

#[derive(Debug, Clone)]
struct Definition<S> {
    value: Option<String>,
    source: S,
    precedence: i32,
    order: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterpolationReference<S> {
    pub variable: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<S>,
}

#[derive(Debug, Clone)]
pub struct InterpolationResult<S> {
    pub value: String,
    pub references: Vec<InterpolationReference<S>>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
pub struct InterpolationContext<S> {
    definitions: HashMap<String, Vec<Definition<S>>>,
    next_order: usize,
}

/// One definition of a variable in the interpolation context, for --debug.
#[derive(Debug, Clone)]
pub struct InterpolationDebugEntry<S> {
    pub value: Option<String>,
    pub source: S,
    pub precedence: i32,
    pub order: usize,
}

impl<S> Default for InterpolationResult<S> {
    fn default() -> Self {
        Self {
            value: String::new(),
            references: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

impl<S> Default for InterpolationContext<S> {
    fn default() -> Self {
        Self {
            definitions: HashMap::new(),
            next_order: 0,
        }
    }
}

impl<S: Clone> InterpolationContext<S> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn define(&mut self, key: String, value: Option<String>, source: S, precedence: i32) {
        let definition = Definition {
            value,
            source,
            precedence,
            order: self.next_order,
        };
        self.next_order += 1;
        self.definitions.entry(key).or_default().push(definition);
    }

    /// All definitions of a variable, lowest precedence first — the
    /// resolution trace behind `explain --debug`.
    pub fn debug(&self, key: &str) -> Vec<InterpolationDebugEntry<S>> {
        self.definitions
            .get(key)
            .map(|definitions| {
                let mut entries: Vec<InterpolationDebugEntry<S>> = definitions
                    .iter()
                    .map(|definition| InterpolationDebugEntry {
                        value: definition.value.clone(),
                        source: definition.source.clone(),
                        precedence: definition.precedence,
                        order: definition.order,
                    })
                    .collect();
                entries.sort_by_key(|entry| (entry.precedence, entry.order));
                entries
            })
            .unwrap_or_default()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.selected(key)
            .and_then(|definition| definition.value.as_ref())
            .is_some()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.selected(key)
            .and_then(|definition| definition.value.as_deref())
    }

    pub fn source(&self, key: &str) -> Option<&S> {
        self.selected(key).map(|definition| &definition.source)
    }

    pub fn interpolate_if(&self, input: &str, enabled: bool) -> InterpolationResult<S> {
        if enabled {
            self.interpolate(input)
        } else {
            InterpolationResult {
                value: input.to_string(),
                ..InterpolationResult::default()
            }
        }
    }

    pub fn interpolate(&self, input: &str) -> InterpolationResult<S> {
        let mut result = InterpolationResult::default();
        let chars: Vec<char> = input.chars().collect();
        let mut index = 0;

        while index < chars.len() {
            if chars[index] != '$' {
                result.value.push(chars[index]);
                index += 1;
                continue;
            }

            if chars.get(index + 1) == Some(&'$') {
                result.value.push('$');
                index += 2;
                continue;
            }

            if chars.get(index + 1) == Some(&'{') {
                match find_matching_brace(&chars, index + 1) {
                    Some(end) => {
                        let expression: String = chars[index + 2..end].iter().collect();
                        self.evaluate_expression(&expression, &mut result);
                        index = end + 1;
                    }
                    None => {
                        result.value.push('$');
                        result.diagnostics.push(Diagnostic::new(
                            Severity::Error,
                            "unterminated-interpolation",
                            format!("unterminated interpolation in '{input}'"),
                        ));
                        index += 1;
                    }
                }
                continue;
            }

            let start = index + 1;
            let mut end = start;
            if chars
                .get(start)
                .is_some_and(|ch| ch.is_ascii_alphabetic() || *ch == '_')
            {
                end += 1;
                while chars
                    .get(end)
                    .is_some_and(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                {
                    end += 1;
                }
            }

            if end == start {
                result.value.push('$');
                index += 1;
            } else {
                let variable: String = chars[start..end].iter().collect();
                self.append_lookup(&variable, &mut result, true);
                index = end;
            }
        }

        result
    }

    fn selected(&self, key: &str) -> Option<&Definition<S>> {
        self.definitions
            .get(key)?
            .iter()
            .max_by_key(|definition| (definition.precedence, definition.order))
    }

    fn append_lookup(&self, variable: &str, result: &mut InterpolationResult<S>, warn: bool) {
        let definition = self.selected(variable);
        result.references.push(InterpolationReference {
            variable: variable.to_string(),
            source: definition.map(|definition| definition.source.clone()),
        });
        if let Some(value) = definition.and_then(|definition| definition.value.as_deref()) {
            result.value.push_str(value);
        } else if warn {
            result.diagnostics.push(Diagnostic::new(
                Severity::Warning,
                "undefined-interpolation-variable",
                format!("{variable} is undefined and resolves to an empty string"),
            ));
        }
    }

    fn evaluate_expression(&self, expression: &str, result: &mut InterpolationResult<S>) {
        let (variable, operator, word) = split_expression(expression);
        if variable.is_empty()
            || !variable.chars().enumerate().all(|(index, ch)| {
                ch == '_' || ch.is_ascii_alphanumeric() && (index > 0 || !ch.is_ascii_digit())
            })
        {
            result.diagnostics.push(Diagnostic::new(
                Severity::Error,
                "invalid-interpolation-expression",
                format!("invalid interpolation expression '${{{expression}}}'"),
            ));
            return;
        }

        let selected = self.selected(variable);
        let is_set = selected
            .and_then(|definition| definition.value.as_ref())
            .is_some();
        let is_nonempty = selected
            .and_then(|definition| definition.value.as_deref())
            .is_some_and(|value| !value.is_empty());
        result.references.push(InterpolationReference {
            variable: variable.to_string(),
            source: selected.map(|definition| definition.source.clone()),
        });

        let selected_value = selected
            .and_then(|definition| definition.value.as_deref())
            .unwrap_or_default();
        match operator {
            None => {
                if is_set {
                    result.value.push_str(selected_value);
                } else {
                    result.diagnostics.push(Diagnostic::new(
                        Severity::Warning,
                        "undefined-interpolation-variable",
                        format!("{variable} is undefined and resolves to an empty string"),
                    ));
                }
            }
            Some(":-") if !is_nonempty => self.append_word(word, result),
            Some("-") if !is_set => self.append_word(word, result),
            Some(":?") if !is_nonempty => result.diagnostics.push(Diagnostic::new(
                Severity::Error,
                "required-interpolation-variable",
                if word.is_empty() {
                    format!("{variable} must be set and non-empty")
                } else {
                    word.to_string()
                },
            )),
            Some("?") if !is_set => result.diagnostics.push(Diagnostic::new(
                Severity::Error,
                "required-interpolation-variable",
                if word.is_empty() {
                    format!("{variable} must be set")
                } else {
                    word.to_string()
                },
            )),
            Some(":+") if is_nonempty => self.append_word(word, result),
            Some("+") if is_set => self.append_word(word, result),
            Some(":+") | Some("+") => {}
            Some(":?") | Some("?") | Some(":-") | Some("-") => {
                result.value.push_str(selected_value)
            }
            Some(other) => result.diagnostics.push(Diagnostic::new(
                Severity::Error,
                "unsupported-interpolation-operator",
                format!("unsupported interpolation operator '{other}'"),
            )),
        }
    }

    fn append_word(&self, word: &str, result: &mut InterpolationResult<S>) {
        let nested = self.interpolate(word);
        result.value.push_str(&nested.value);
        result.references.extend(nested.references);
        result.diagnostics.extend(nested.diagnostics);
    }
}

fn find_matching_brace(chars: &[char], opening: usize) -> Option<usize> {
    let mut depth = 0;
    for (index, ch) in chars.iter().enumerate().skip(opening) {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_expression(expression: &str) -> (&str, Option<&str>, &str) {
    let mut boundary = expression.len();
    for (index, ch) in expression.char_indices() {
        if ch == ':' || ch == '-' || ch == '?' || ch == '+' {
            boundary = index;
            break;
        }
    }
    if boundary == expression.len() {
        return (expression, None, "");
    }

    let rest = &expression[boundary..];
    for operator in [":-", ":?", ":+", "-", "?", "+"] {
        if let Some(word) = rest.strip_prefix(operator) {
            return (&expression[..boundary], Some(operator), word);
        }
    }
    (&expression[..boundary], Some(rest), "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SourceKind, SourceRef};

    type TestContext = InterpolationContext<crate::model::SourceRef>;

    fn context(values: &[(&str, Option<&str>)]) -> TestContext {
        let mut context = InterpolationContext::new();
        for (index, (key, value)) in values.iter().enumerate() {
            context.define(
                (*key).to_string(),
                value.map(str::to_string),
                SourceRef::new(SourceKind::HostEnvironment, None, None, "test"),
                index as i32,
            );
        }
        context
    }

    #[test]
    fn expands_supported_parameter_forms() {
        let context = context(&[("SET", Some("yes")), ("EMPTY", Some(""))]);
        let result = context.interpolate(
            "$SET/${MISSING:-fallback}/${EMPTY-default}/${EMPTY:-fallback}/${SET:+alt}/$$HOME",
        );
        assert_eq!(result.value, "yes/fallback//fallback/alt/$HOME");
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn reports_required_and_undefined_variables() {
        let context = context(&[]);
        let result = context.interpolate("${REQUIRED:?set it}/$MISSING");
        assert_eq!(result.value, "/");
        assert_eq!(result.diagnostics.len(), 2);
        assert_eq!(result.diagnostics[0].severity, Severity::Error);
    }
}
