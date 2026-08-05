use std::fmt::Write;

use sha2::{Digest, Sha256};

use crate::model::{
    CandidateDisposition, Diagnostic, DockerStatus, Explanation, ProjectReport, ServiceReport,
    Severity, SourceKind, SourceRef, VariableState,
};

pub fn project_human(report: &ProjectReport, show_values: bool) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "EnvOrigin {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(output, "compose: {}", report.compose_file.display());
    let _ = writeln!(
        output,
        "docker verification: {}",
        docker_label(&report.docker_status)
    );
    if report.interpolation_files.is_empty() {
        let _ = writeln!(output, "interpolation files: none");
    } else {
        let files = report
            .interpolation_files
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(output, "interpolation files: {files}");
    }

    for service in &report.services {
        let present = service
            .variables
            .iter()
            .filter(|variable| variable.state == VariableState::Present)
            .count();
        let _ = writeln!(
            output,
            "\nservice {} ({} present, {} tracked)",
            service.name,
            present,
            service.variables.len()
        );
        if service.variables.is_empty() {
            let _ = writeln!(output, "  no Compose-managed environment variables");
        }
        for variable in &service.variables {
            let state = match variable.state {
                VariableState::Present => "set",
                VariableState::Absent => "absent",
            };
            let source = variable
                .winner
                .as_ref()
                .map(source_summary)
                .unwrap_or_else(|| "unknown source".to_string());
            let rendered = variable
                .value
                .as_deref()
                .map(|value| display_value(value, show_values))
                .unwrap_or_else(|| "—".to_string());
            let _ = writeln!(
                output,
                "  {:<24} {:<7} {:<24} ← {}",
                variable.variable, state, rendered, source
            );
            for candidate in &variable.candidates {
                // Dead-code hint: the candidate lost to a higher-precedence
                // layer with a different value, so it can be removed safely.
                if candidate.disposition == CandidateDisposition::Shadowed
                    && candidate.value.as_deref() != variable.value.as_deref()
                {
                    let _ = writeln!(
                        output,
                        "    note: {} is shadowed by {} (different value)",
                        source_summary(&candidate.source),
                        source
                    );
                }
            }
        }
    }

    append_diagnostics(&mut output, &report.diagnostics);
    output
}

pub fn service_human(report: &ProjectReport, service: &ServiceReport, show_values: bool) -> String {
    let mut filtered = report.clone();
    filtered.services = vec![service.clone()];
    project_human(&filtered, show_values)
}

pub fn explanation_human(explanation: &Explanation, show_values: bool) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "{} for service {}",
        explanation.variable, explanation.service
    );
    let _ = writeln!(
        output,
        "state: {}",
        match explanation.state {
            VariableState::Present => "present",
            VariableState::Absent => "absent",
        }
    );
    if let Some(value) = &explanation.value {
        let _ = writeln!(output, "value: {}", display_value(value, show_values));
    }
    if let Some(winner) = &explanation.winner {
        let _ = writeln!(output, "winner: {}", source_summary(winner));
    }
    if !explanation.derived_from.is_empty() {
        let _ = writeln!(output, "derived from:");
        for source in &explanation.derived_from {
            let _ = writeln!(output, "  - {}", source_summary(source));
        }
    }
    if !explanation.candidates.is_empty() {
        let _ = writeln!(output, "candidates:");
        for candidate in &explanation.candidates {
            let marker = match candidate.disposition {
                CandidateDisposition::Winner => "winner",
                CandidateDisposition::Shadowed => "shadowed",
                CandidateDisposition::Derived => "derived",
                CandidateDisposition::Removed => "removed",
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
    append_diagnostics(&mut output, &explanation.diagnostics);
    output
}

pub fn project_json(report: &ProjectReport, show_values: bool) -> String {
    let report = if show_values {
        report.clone()
    } else {
        redact_project(report)
    };
    serde_json::to_string_pretty(&report).expect("ProjectReport is serializable")
}

pub fn explanation_json(explanation: &Explanation, show_values: bool) -> String {
    let mut explanation = explanation.clone();
    if !show_values {
        redact_explanation(&mut explanation);
    }
    serde_json::to_string_pretty(&explanation).expect("Explanation is serializable")
}

fn redact_project(report: &ProjectReport) -> ProjectReport {
    let mut report = report.clone();
    for service in &mut report.services {
        for explanation in &mut service.variables {
            redact_explanation(explanation);
        }
    }
    report
}

fn redact_explanation(explanation: &mut Explanation) {
    if let Some(value) = &mut explanation.value {
        *value = fingerprint(value);
    }
    for candidate in &mut explanation.candidates {
        if let Some(value) = &mut candidate.value {
            *value = fingerprint(value);
        }
    }
}

fn append_diagnostics(output: &mut String, diagnostics: &[Diagnostic]) {
    if diagnostics.is_empty() {
        return;
    }
    let _ = writeln!(output, "\ndiagnostics:");
    for diagnostic in diagnostics {
        let severity = match diagnostic.severity {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        let _ = writeln!(
            output,
            "  - {severity} [{}]: {}",
            diagnostic.code, diagnostic.message
        );
    }
}

fn source_summary(source: &SourceRef) -> String {
    let kind = match source.kind {
        SourceKind::HostEnvironment => "host environment",
        SourceKind::DefaultEnvFile => "default .env",
        SourceKind::CliEnvFile => "--env-file",
        SourceKind::ServiceEnvFile => "service env_file",
        SourceKind::ServiceEnvironment => "service environment",
        SourceKind::DockerCanonical => "Docker canonical model",
    };
    format!("{kind} ({})", source.label())
}

fn display_value(value: &str, show_values: bool) -> String {
    if show_values {
        format!("{value:?}")
    } else {
        fingerprint(value)
    }
}

pub(crate) fn fingerprint(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut short = String::with_capacity(8);
    for byte in &digest[..4] {
        use std::fmt::Write;
        let _ = write!(short, "{byte:02x}");
    }
    format!("<redacted sha256:{short}>")
}

fn docker_label(status: &DockerStatus) -> &'static str {
    match status {
        DockerStatus::Verified => "verified",
        DockerStatus::Unavailable => "unavailable",
        DockerStatus::Failed => "failed",
        DockerStatus::Skipped => "skipped",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_are_stable_and_do_not_reveal_values() {
        let one = fingerprint("super-secret");
        let two = fingerprint("super-secret");
        assert_eq!(one, two);
        assert!(!one.contains("super-secret"));
    }
}
