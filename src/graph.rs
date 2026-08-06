//! Provenance graph rendering (mermaid flowchart).
//!
//! A one-glance view of where each variable comes from: solid edges point at
//! the winning source, dashed edges at shadowed candidates, dotted edges at
//! interpolation dependencies. Pipe the output into mermaid-cli or any
//! mermaid renderer.
//!
//! Structural note: mermaid forbids referencing a subgraph's own id from
//! inside it ("setting X as parent of X would create a cycle"), so all
//! source nodes and edges are emitted *after* the subgraph blocks close.

use std::collections::BTreeMap;

use crate::actions::{ActionsDisposition, ActionsReport, ActionsSourceRef, ActionsVariable};
use crate::model::{CandidateDisposition, ProjectReport, SourceRef, VariableState};

const HEADER: &str = "graph LR\n";

fn mermaid_label(text: &str) -> String {
    let escaped = text.replace('"', "'");
    format!("\"{escaped}\"")
}

struct GraphBuilder {
    /// Structural lines (subgraph blocks and in-block variable nodes).
    lines: Vec<String>,
    /// Source-node declarations and edges, flushed after subgraphs close.
    pending: Vec<String>,
    source_ids: BTreeMap<String, String>,
    next_source: usize,
}

impl GraphBuilder {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            pending: Vec::new(),
            source_ids: BTreeMap::new(),
            next_source: 0,
        }
    }

    fn source_id(&mut self, label: &str) -> String {
        if let Some(id) = self.source_ids.get(label) {
            return id.clone();
        }
        let id = format!("src_{}", self.next_source);
        self.next_source += 1;
        self.source_ids.insert(label.to_string(), id.clone());
        self.pending
            .push(format!("  {id}[{}]", mermaid_label(label)));
        id
    }

    fn variable(&mut self, var_id: &str, name: &str, state: VariableState) {
        let label = if state == VariableState::Present {
            name.to_string()
        } else {
            format!("{name} (absent)")
        };
        self.lines
            .push(format!("  {var_id}[{}]", mermaid_label(&label)));
    }

    fn edge(&mut self, from: &str, to: &str, style: &str, label: &str) {
        self.pending
            .push(format!("  {from} {style}|{}| {to}", mermaid_label(label)));
    }

    /// Close a subgraph and flush source nodes + edges outside of it.
    fn end_subgraph(&mut self) {
        self.lines.push("  end".to_string());
        self.lines.append(&mut self.pending);
    }

    fn finish(self) -> String {
        format!("{HEADER}{}\n", self.lines.join("\n"))
    }
}

fn safe_id(text: &str) -> String {
    text.replace(|c: char| !c.is_alphanumeric(), "_")
}

pub fn project_graph(report: &ProjectReport) -> String {
    let mut builder = GraphBuilder::new();
    for service in &report.services {
        let service_id = format!("svc_{}", safe_id(&service.name));
        builder.lines.push(format!(
            "  subgraph {service_id}[{}]",
            mermaid_label(&service.name)
        ));
        for variable in &service.variables {
            let var_id = format!(
                "var_{}_{}",
                safe_id(&service.name),
                safe_id(&variable.variable)
            );
            builder.variable(&var_id, &variable.variable, variable.state);
            if let Some(winner) = &variable.winner {
                let source_id = builder.source_id(&source_label(winner));
                builder.edge(&var_id, &source_id, "-->", &winner.kind_label());
            }
            for candidate in &variable.candidates {
                if candidate.disposition == CandidateDisposition::Shadowed {
                    let source_id = builder.source_id(&source_label(&candidate.source));
                    builder.edge(&var_id, &source_id, "-.->", "shadowed");
                }
            }
            for derived in &variable.derived_from {
                let source_id = builder.source_id(&source_label(derived));
                builder.edge(&var_id, &source_id, "-.->", "derived");
            }
        }
        builder.end_subgraph();
    }
    builder.finish()
}

pub fn gitlab_graph(report: &crate::gitlab::GitlabReport) -> String {
    let mut builder = GraphBuilder::new();
    let globals_id = "globals";
    builder.lines.push(format!(
        "  subgraph {globals_id}[{}]",
        mermaid_label("global variables")
    ));
    for variable in &report.global_variables {
        let var_id = format!("var_global_{}", safe_id(&variable.variable));
        builder.variable(&var_id, &variable.variable, variable.state);
        add_source_edges(
            &mut builder,
            &var_id,
            variable.winner.as_ref().map(|winner| winner.label()),
            variable
                .candidates
                .iter()
                .filter(|candidate| {
                    candidate.disposition == crate::gitlab::GitlabDisposition::Shadowed
                })
                .map(|candidate| candidate.source.label()),
        );
    }
    builder.end_subgraph();
    for job in &report.jobs {
        let job_id = format!("job_{}", safe_id(&job.id));
        builder.lines.push(format!(
            "  subgraph {job_id}[{}]",
            mermaid_label(&format!("job {}", job.id))
        ));
        for variable in &job.variables {
            let var_id = format!("var_{}_{}", job_id, safe_id(&variable.variable));
            builder.variable(&var_id, &variable.variable, variable.state);
            add_source_edges(
                &mut builder,
                &var_id,
                variable.winner.as_ref().map(|winner| winner.label()),
                variable
                    .candidates
                    .iter()
                    .filter(|candidate| {
                        candidate.disposition == crate::gitlab::GitlabDisposition::Shadowed
                    })
                    .map(|candidate| candidate.source.label()),
            );
        }
        builder.end_subgraph();
    }
    builder.finish()
}

pub fn circleci_graph(report: &crate::circleci::CircleciReport) -> String {
    let mut builder = GraphBuilder::new();
    for job in &report.jobs {
        let job_id = format!("job_{}", safe_id(&job.id));
        builder.lines.push(format!(
            "  subgraph {job_id}[{}]",
            mermaid_label(&format!("job {}", job.id))
        ));
        for variable in &job.variables {
            let var_id = format!("var_{}_{}", job_id, safe_id(&variable.variable));
            builder.variable(&var_id, &variable.variable, variable.state);
            add_source_edges(
                &mut builder,
                &var_id,
                variable.winner.as_ref().map(|winner| winner.label()),
                variable
                    .candidates
                    .iter()
                    .filter(|candidate| {
                        candidate.disposition == crate::circleci::CircleciDisposition::Shadowed
                    })
                    .map(|candidate| candidate.source.label()),
            );
        }
        builder.end_subgraph();
    }
    builder.finish()
}

/// Shared edges for the GitLab/CircleCI graphs: winner (solid) and
/// shadowed candidates (dashed). Labels carry no kind information — the
/// source types are backend-specific.
fn add_source_edges(
    builder: &mut GraphBuilder,
    var_id: &str,
    winner_label: Option<String>,
    shadowed: impl Iterator<Item = String>,
) {
    if let Some(label) = winner_label {
        let id = builder.source_id(&label);
        builder.edge(var_id, &id, "-->", "winner");
    }
    for label in shadowed {
        let id = builder.source_id(&label);
        builder.edge(var_id, &id, "-.->", "shadowed");
    }
}

pub fn actions_graph(report: &ActionsReport) -> String {
    let mut builder = GraphBuilder::new();
    for job in &report.jobs {
        let job_id = format!("job_{}", safe_id(&job.id));
        builder.lines.push(format!(
            "  subgraph {job_id}[{}]",
            mermaid_label(&format!("job {}", job.id))
        ));
        for variable in &job.variables {
            let var_id = format!("var_{}_{}", job_id, safe_id(&variable.variable));
            builder.variable(&var_id, &variable.variable, variable.state);
            add_actions_edges(&mut builder, &var_id, variable);
        }
        for step in &job.steps {
            let step_id = format!("step_{}_{}", job_id, step.index);
            let label = step
                .name
                .clone()
                .or_else(|| step.uses.clone())
                .unwrap_or_else(|| format!("step #{}", step.index));
            builder
                .lines
                .push(format!("  {step_id}[{}]", mermaid_label(&label)));
            for variable in &step.variables {
                let var_id = format!("{step_id}_var_{}", safe_id(&variable.variable));
                builder.variable(&var_id, &variable.variable, variable.state);
                add_actions_edges(&mut builder, &var_id, variable);
            }
        }
        builder.end_subgraph();
    }
    builder.finish()
}

fn add_actions_edges(builder: &mut GraphBuilder, var_id: &str, variable: &ActionsVariable) {
    if let Some(winner) = &variable.winner {
        let source_id = builder.source_id(&actions_source_label(winner));
        builder.edge(var_id, &source_id, "-->", &winner.kind_label());
    }
    for candidate in &variable.candidates {
        if candidate.disposition == ActionsDisposition::Shadowed {
            let source_id = builder.source_id(&actions_source_label(&candidate.source));
            builder.edge(var_id, &source_id, "-.->", "shadowed");
        }
    }
}

trait KindLabel {
    fn kind_label(&self) -> String;
}

impl KindLabel for SourceRef {
    fn kind_label(&self) -> String {
        format!("{:?}", self.kind)
    }
}

impl KindLabel for ActionsSourceRef {
    fn kind_label(&self) -> String {
        format!("{:?}", self.kind)
    }
}

fn source_label(source: &SourceRef) -> String {
    source.label()
}

fn actions_source_label(source: &ActionsSourceRef) -> String {
    source.label()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions;
    use crate::circleci;
    use crate::gitlab;
    use crate::model::VariableState;
    use std::path::{Path, PathBuf};

    fn compose_report() -> ProjectReport {
        // Minimal report built by hand: one service, one variable with a
        // winner source and a shadowed candidate.
        let source = |kind: SourceRef| kind;
        let winner = source(SourceRef::new(
            crate::model::SourceKind::ServiceEnvironment,
            Some(PathBuf::from("compose.yaml")),
            Some(9),
            "service environment",
        ));
        let shadowed = SourceRef::new(
            crate::model::SourceKind::ServiceEnvFile,
            Some(PathBuf::from("env/web.env")),
            Some(1),
            "service env_file",
        );
        ProjectReport {
            compose_file: PathBuf::from("compose.yaml"),
            project_directory: PathBuf::from("."),
            interpolation_files: Vec::new(),
            docker_status: crate::model::DockerStatus::Skipped,
            services: vec![crate::model::ServiceReport {
                name: "web".to_string(),
                diagnostics: Vec::new(),
                variables: vec![crate::model::Explanation {
                    variable: "P".to_string(),
                    service: "web".to_string(),
                    state: VariableState::Present,
                    value: Some("v".to_string()),
                    winner: Some(winner.clone()),
                    derived_from: Vec::new(),
                    candidates: vec![
                        crate::model::Candidate {
                            source: shadowed,
                            value: Some("old".to_string()),
                            disposition: crate::model::CandidateDisposition::Shadowed,
                        },
                        crate::model::Candidate {
                            source: winner,
                            value: Some("v".to_string()),
                            disposition: crate::model::CandidateDisposition::Winner,
                        },
                    ],
                    diagnostics: Vec::new(),
                }],
            }],
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn compose_graph_has_service_subgraph_and_edges() {
        let graph = project_graph(&compose_report());
        assert!(graph.starts_with("graph LR"));
        assert!(graph.contains("subgraph svc_web[\"web\"]"));
        assert!(graph.contains("var_web_P[\"P\"]"));
        assert!(graph.contains("-->|\"ServiceEnvironment\"|"));
        assert!(graph.contains("-.->|\"shadowed\"|"));
        assert!(graph.contains("end"));
    }

    #[test]
    fn absent_variables_are_marked() {
        let mut report = compose_report();
        report.services[0].variables[0].state = VariableState::Absent;
        let graph = project_graph(&report);
        assert!(graph.contains("P (absent)"));
    }

    #[test]
    fn workflow_graph_covers_jobs_steps_and_sources() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/actions/.github/workflows/workflow.yaml");
        let report = actions::analyze_workflow(&path, None).unwrap();
        let graph = actions_graph(&report);
        assert!(graph.contains("subgraph job_build"));
        assert!(graph.contains("step_"));
        assert!(graph.contains("-->|\"StepEnv\"|") || graph.contains("-->|\"JobEnv\"|"));
    }

    #[test]
    fn gitlab_and_circleci_graphs_render() {
        let gitlab_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gitlab/.gitlab-ci.yml");
        let report = gitlab::analyze_gitlab(&gitlab_path).unwrap();
        let graph = gitlab_graph(&report);
        assert!(graph.contains("subgraph globals"));
        assert!(graph.contains("subgraph job_build"));

        let circle_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/circleci/.circleci/config.yml");
        let report = circleci::analyze_circleci(&circle_path).unwrap();
        let graph = circleci_graph(&report);
        assert!(graph.contains("subgraph job_build"));
    }
}
