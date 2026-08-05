//! LSP server: hover, go-to-definition, and live diagnostics for the four
//! configuration backends.
//!
//! The server routes each opened document to the matching analyzer by file
//! name. Analysis reads the file from disk (unsaved buffer edits are not
//! analyzed yet — a documented v1 limitation). Diagnostics mirror the
//! `audit` findings per variable: undefined interpolation references,
//! shadowed dead-code lines, sensitive values, and analyzer warnings.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService};

use crate::actions;
use crate::circleci;
use crate::gitlab;
use crate::model::{Diagnostic as EnvDiagnostic, Severity as EnvSeverity};
use crate::{analyze, AnalyzeOptions};

#[derive(Debug, Clone)]
struct Symbol {
    name: String,
    /// 1-based line of the variable definition in the analyzed document.
    line: usize,
    state: String,
    /// Redacted or expression-form value.
    value: Option<String>,
    winner_label: String,
    winner_path: Option<std::path::PathBuf>,
    winner_line: Option<usize>,
}

#[derive(Debug, Clone)]
struct LspDiag {
    line: Option<usize>,
    severity: DiagnosticSeverity,
    code: String,
    message: String,
}

type SymbolTuple = (
    String,
    usize,
    String,
    Option<String>,
    String,
    Option<std::path::PathBuf>,
    Option<usize>,
);

#[derive(Debug, Clone, Default)]
struct LspAnalysis {
    symbols: Vec<Symbol>,
    diagnostics: Vec<LspDiag>,
}

pub struct Backend {
    client: Client,
    cache: Mutex<HashMap<Url, LspAnalysis>>,
}

fn severity(severity: EnvSeverity) -> DiagnosticSeverity {
    match severity {
        EnvSeverity::Info => DiagnosticSeverity::INFORMATION,
        EnvSeverity::Warning => DiagnosticSeverity::WARNING,
        EnvSeverity::Error => DiagnosticSeverity::ERROR,
    }
}

fn from_env_diag(diagnostic: &EnvDiagnostic) -> LspDiag {
    LspDiag {
        line: None,
        severity: severity(diagnostic.severity),
        code: diagnostic.code.clone(),
        message: diagnostic.message.clone(),
    }
}

/// Route a file to its analyzer and extract symbols + diagnostics.
fn analyze_file(path: &Path) -> std::result::Result<LspAnalysis, Box<dyn std::error::Error>> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let parent = path
        .parent()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    let workflow_dir = parent.ends_with(".github/workflows");
    let is_compose = matches!(
        file_name.as_str(),
        "compose.yaml" | "compose.yml" | "docker-compose.yaml" | "docker-compose.yml"
    );

    let mut analysis = LspAnalysis::default();
    let mut collect = |symbols: Vec<SymbolTuple>, diagnostics: Vec<LspDiag>| {
        analysis.symbols = symbols
            .into_iter()
            .map(
                |(name, line, state, value, winner_label, winner_path, winner_line)| Symbol {
                    name,
                    line,
                    state,
                    value,
                    winner_label,
                    winner_path,
                    winner_line,
                },
            )
            .collect();
        analysis.diagnostics = diagnostics;
    };

    if is_compose {
        let report = analyze(&AnalyzeOptions {
            compose_file: path.to_path_buf(),
            docker_check: false,
            ..AnalyzeOptions::default()
        })?;
        let mut symbols = Vec::new();
        let mut diagnostics = Vec::new();
        for diagnostic in &report.diagnostics {
            diagnostics.push(from_env_diag(diagnostic));
        }
        for service in &report.services {
            for variable in &service.variables {
                let line = variable
                    .winner
                    .as_ref()
                    .and_then(|winner| winner.line)
                    .unwrap_or(1);
                let winner_label = variable
                    .winner
                    .as_ref()
                    .map(|winner| format!("{:?} ({})", winner.kind, winner.label()))
                    .unwrap_or_else(|| "unknown source".to_string());
                let (winner_path, winner_line) = variable
                    .winner
                    .as_ref()
                    .map(|winner| (winner.path.clone(), winner.line))
                    .unwrap_or((None, None));
                symbols.push((
                    variable.variable.clone(),
                    line,
                    match variable.state {
                        crate::model::VariableState::Present => "set".to_string(),
                        crate::model::VariableState::Absent => "absent".to_string(),
                    },
                    variable.value.clone(),
                    winner_label,
                    winner_path,
                    winner_line,
                ));
                for diagnostic in &variable.diagnostics {
                    let mut diag = from_env_diag(diagnostic);
                    diag.line = Some(line);
                    diagnostics.push(diag);
                }
            }
        }
        collect(symbols, diagnostics);
    } else if workflow_dir && file_name.ends_with(".yml") {
        let report = actions::analyze_workflow(path, None)?;
        let mut symbols = Vec::new();
        let mut diagnostics = Vec::new();
        for diagnostic in &report.diagnostics {
            diagnostics.push(from_env_diag(diagnostic));
        }
        for job in &report.jobs {
            for variable in job
                .variables
                .iter()
                .chain(job.steps.iter().flat_map(|step| step.variables.iter()))
            {
                let line = variable
                    .winner
                    .as_ref()
                    .and_then(|winner| winner.line)
                    .unwrap_or(1);
                let winner_label = variable
                    .winner
                    .as_ref()
                    .map(|winner| format!("{:?} ({})", winner.kind, winner.label()))
                    .unwrap_or_else(|| "unknown source".to_string());
                let (winner_path, winner_line) = variable
                    .winner
                    .as_ref()
                    .map(|winner| (winner.path.clone(), winner.line))
                    .unwrap_or((None, None));
                symbols.push((
                    variable.variable.clone(),
                    line,
                    match variable.state {
                        crate::model::VariableState::Present => "set".to_string(),
                        crate::model::VariableState::Absent => "absent".to_string(),
                    },
                    variable.value.clone(),
                    winner_label,
                    winner_path,
                    winner_line,
                ));
                for diagnostic in &variable.diagnostics {
                    let mut diag = from_env_diag(diagnostic);
                    diag.line = Some(line);
                    diagnostics.push(diag);
                }
            }
        }
        collect(symbols, diagnostics);
    } else if file_name == ".gitlab-ci.yml" {
        let report = gitlab::analyze_gitlab(path)?;
        let mut symbols = Vec::new();
        let mut diagnostics = Vec::new();
        for diagnostic in &report.diagnostics {
            diagnostics.push(from_env_diag(diagnostic));
        }
        let mut all: Vec<&gitlab::GitlabVariable> = report
            .global_variables
            .iter()
            .chain(report.jobs.iter().flat_map(|job| job.variables.iter()))
            .collect();
        all.sort_by_key(|variable| {
            variable
                .winner
                .as_ref()
                .and_then(|winner| winner.line)
                .unwrap_or(1)
        });
        all.dedup_by_key(|variable| variable.variable.clone());
        for variable in all {
            let line = variable
                .winner
                .as_ref()
                .and_then(|winner| winner.line)
                .unwrap_or(1);
            let winner_label = variable
                .winner
                .as_ref()
                .map(|winner| format!("{:?} ({})", winner.kind, winner.label()))
                .unwrap_or_else(|| "unknown source".to_string());
            let (winner_path, winner_line) = variable
                .winner
                .as_ref()
                .map(|winner| (winner.path.clone(), winner.line))
                .unwrap_or((None, None));
            symbols.push((
                variable.variable.clone(),
                line,
                match variable.state {
                    crate::model::VariableState::Present => "set".to_string(),
                    crate::model::VariableState::Absent => "absent".to_string(),
                },
                variable.value.clone(),
                winner_label,
                winner_path,
                winner_line,
            ));
            for diagnostic in &variable.diagnostics {
                let mut diag = from_env_diag(diagnostic);
                diag.line = Some(line);
                diagnostics.push(diag);
            }
        }
        collect(symbols, diagnostics);
    } else if file_name == "config.yml" && parent.ends_with(".circleci") {
        let report = circleci::analyze_circleci(path)?;
        let mut symbols = Vec::new();
        let mut diagnostics = Vec::new();
        for diagnostic in &report.diagnostics {
            diagnostics.push(from_env_diag(diagnostic));
        }
        for job in &report.jobs {
            for variable in &job.variables {
                let line = variable
                    .winner
                    .as_ref()
                    .and_then(|winner| winner.line)
                    .unwrap_or(1);
                let winner_label = variable
                    .winner
                    .as_ref()
                    .map(|winner| format!("{:?} ({})", winner.kind, winner.label()))
                    .unwrap_or_else(|| "unknown source".to_string());
                let (winner_path, winner_line) = variable
                    .winner
                    .as_ref()
                    .map(|winner| (winner.path.clone(), winner.line))
                    .unwrap_or((None, None));
                symbols.push((
                    variable.variable.clone(),
                    line,
                    match variable.state {
                        crate::model::VariableState::Present => "set".to_string(),
                        crate::model::VariableState::Absent => "absent".to_string(),
                    },
                    variable.value.clone(),
                    winner_label,
                    winner_path,
                    winner_line,
                ));
                for diagnostic in &variable.diagnostics {
                    let mut diag = from_env_diag(diagnostic);
                    diag.line = Some(line);
                    diagnostics.push(diag);
                }
            }
        }
        collect(symbols, diagnostics);
    } else {
        return Ok(LspAnalysis::default());
    }
    Ok(analysis)
}

fn symbol_at_line(analysis: &LspAnalysis, line: u32) -> Option<&Symbol> {
    analysis
        .symbols
        .iter()
        .find(|symbol| symbol.line == line as usize + 1)
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        let _ = self
            .client
            .log_message(MessageType::INFO, "envorigin LSP ready")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.analyze_and_publish(&params.text_document.uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.analyze_and_publish(&params.text_document.uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.analyze_and_publish(&params.text_document.uri).await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let cache = self.cache.lock().unwrap();
        let Some(analysis) = cache.get(&uri) else {
            return Ok(None);
        };
        let Some(symbol) = symbol_at_line(analysis, position.line) else {
            return Ok(None);
        };
        let value = symbol
            .value
            .as_deref()
            .map(|value| format!(" = `{value}`"))
            .unwrap_or_default();
        let markdown = format!(
            "`{}` · {}{}\n\n← {}",
            symbol.name, symbol.state, value, symbol.winner_label
        );
        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: markdown,
            }),
            range: Some(Range {
                start: Position {
                    line: position.line,
                    character: 0,
                },
                end: Position {
                    line: position.line,
                    character: 0,
                },
            }),
        }))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let cache = self.cache.lock().unwrap();
        let Some(analysis) = cache.get(&uri) else {
            return Ok(None);
        };
        let Some(symbol) = symbol_at_line(analysis, position.line) else {
            return Ok(None);
        };
        let Some((path, line)) = symbol.winner_path.as_ref().zip(symbol.winner_line) else {
            return Ok(None);
        };
        let Ok(target_uri) = Url::from_file_path(path) else {
            return Ok(None);
        };
        Ok(Some(GotoDefinitionResponse::Scalar(Location {
            uri: target_uri,
            range: Range {
                start: Position {
                    line: line.saturating_sub(1) as u32,
                    character: 0,
                },
                end: Position {
                    line: line.saturating_sub(1) as u32,
                    character: 0,
                },
            },
        })))
    }
}

impl Backend {
    async fn analyze_and_publish(&self, uri: &Url) {
        let Some(path) = uri.to_file_path().ok() else {
            return;
        };
        let analysis = analyze_file(&path).unwrap_or_default();
        {
            let mut cache = self.cache.lock().unwrap();
            cache.insert(uri.clone(), analysis.clone());
        }
        let diagnostics: Vec<Diagnostic> = analysis
            .diagnostics
            .iter()
            .map(|diag| Diagnostic {
                range: Range {
                    start: Position {
                        line: diag.line.unwrap_or(1).saturating_sub(1) as u32,
                        character: 0,
                    },
                    end: Position {
                        line: diag.line.unwrap_or(1).saturating_sub(1) as u32,
                        character: 0,
                    },
                },
                severity: Some(diag.severity),
                code: Some(NumberOrString::String(diag.code.clone())),
                message: diag.message.clone(),
                ..Default::default()
            })
            .collect();
        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }
}

pub fn run_lsp() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        let (service, socket) = LspService::new(|client| Backend {
            client,
            cache: Mutex::new(HashMap::new()),
        });
        tower_lsp::Server::new(stdin, stdout, socket)
            .serve(service)
            .await;
    });
}
