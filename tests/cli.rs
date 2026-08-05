use std::path::Path;

use assert_cmd::Command;
use envorigin::model::{ProjectReport, VariableState};
use predicates::prelude::*;

fn fixture(sub: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(sub)
        .join("compose.yaml")
        .display()
        .to_string()
}

fn command(subcommand: &str, sub: &str) -> Command {
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .arg(subcommand)
        .arg("--no-docker-check")
        .arg("--file")
        .arg(fixture(sub));
    command
}

#[test]
fn scan_redacts_values_by_default() {
    let output = command("scan", "basic").assert().success();
    output
        .stdout(predicate::str::contains("service web"))
        .stdout(predicate::str::contains("<redacted sha256:"))
        .stdout(predicate::str::contains("production").not());
}

#[test]
fn scan_show_values_reveals_plaintext() {
    let output = command("scan", "basic")
        .arg("--show-values")
        .assert()
        .success();
    output
        .stdout(predicate::str::contains("\"production\""))
        .stdout(predicate::str::contains("<redacted").not());
}

#[test]
fn shell_environment_beats_interpolation_file() {
    let output = command("scan", "basic")
        .arg("--show-values")
        .env("SHELL_OVERRIDE", "from_shell")
        .assert()
        .success();
    output.stdout(predicate::str::contains("from_shell"));
}

#[test]
fn explain_reports_winner_and_shadowed_candidates() {
    let output = command("explain", "precedence")
        .args(["P", "-s", "web", "--show-values"])
        .assert()
        .success();
    output
        .stdout(predicate::str::contains("winner: service environment"))
        .stdout(predicate::str::contains("from_overrides_env"))
        .stdout(predicate::str::contains("from_web_env"))
        .stdout(predicate::str::contains("from_project_env").not());
}

#[test]
fn explain_traces_derived_values_to_their_source() {
    let output = command("explain", "precedence")
        .args(["T", "-s", "web", "--show-values"])
        .assert()
        .success();
    output
        .stdout(predicate::str::contains("derived from:"))
        .stdout(predicate::str::contains("default .env"))
        .stdout(predicate::str::contains(":2"));
}

#[test]
fn explain_reports_absent_variables() {
    let output = command("explain", "precedence")
        .args(["Y", "-s", "web"])
        .assert()
        .success();
    output.stdout(predicate::str::contains("state: absent"));
}

#[test]
fn explain_requires_service_for_multi_service_file() {
    let output = command("explain", "precedence").arg("P").assert().failure();
    output.stderr(predicate::str::contains("--service is required"));
}

#[test]
fn unknown_variable_is_an_error() {
    let output = command("explain", "basic")
        .args(["NOPE", "-s", "web"])
        .assert()
        .failure();
    output.stderr(predicate::str::contains("not configured"));
}

#[test]
fn json_output_has_expected_shape_and_redacts() {
    let output = command("scan", "precedence")
        .arg("--format")
        .arg("json")
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    let report: ProjectReport = serde_json::from_str(&stdout).expect("valid report JSON");

    assert_eq!(report.compose_file, Path::new(&fixture("precedence")));
    assert_eq!(report.services.len(), 2);

    let web = report
        .services
        .iter()
        .find(|service| service.name == "web")
        .expect("web service present");
    let p = web
        .variables
        .iter()
        .find(|variable| variable.variable == "P")
        .expect("P variable present");
    assert_eq!(p.state, VariableState::Present);
    assert_eq!(p.candidates.len(), 3);
    let value = p.value.as_deref().expect("P has a value");
    assert!(
        value.starts_with("<redacted sha256:"),
        "values are redacted by default, got {value:?}"
    );
}

#[test]
fn no_docker_check_marks_verification_skipped() {
    let output = command("scan", "basic").assert().success();
    output.stdout(predicate::str::contains("docker verification: skipped"));
}

// --- GitHub Actions ---

fn workflow_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/actions/.github/workflows/workflow.yaml")
        .display()
        .to_string()
}

fn actions_command(subcommand: &str) -> Command {
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .arg("actions")
        .arg(subcommand)
        .arg("--file")
        .arg(workflow_path());
    command
}

#[test]
fn actions_scan_lists_jobs_steps_and_sources() {
    let output = actions_command("scan").assert().success();
    output
        .stdout(predicate::str::contains("job build"))
        .stdout(predicate::str::contains("step \"Configure\""))
        .stdout(predicate::str::contains("workflow env"))
        .stdout(predicate::str::contains("<redacted sha256:"));
}

#[test]
fn actions_scan_reveals_values_with_flag() {
    let output = actions_command("scan")
        .arg("--show-values")
        .assert()
        .success();
    output
        .stdout(predicate::str::contains("\"step_value\""))
        .stdout(predicate::str::contains("<redacted").not());
}

#[test]
fn actions_explain_shows_full_shadowing_chain() {
    let output = actions_command("explain")
        .args(["SHARED", "-j", "build", "-s", "Configure", "--show-values"])
        .assert()
        .success();
    output
        .stdout(predicate::str::contains("winner: step env"))
        .stdout(predicate::str::contains("[shadowed] workflow env"))
        .stdout(predicate::str::contains("[shadowed] job env"))
        .stdout(predicate::str::contains("workflow.yaml:19"));
}

#[test]
fn actions_explain_resolves_env_file() {
    let output = actions_command("explain")
        .args(["FILE_LEVEL", "-j", "deploy", "--show-values"])
        .assert()
        .success();
    output
        .stdout(predicate::str::contains("winner: env file"))
        .stdout(predicate::str::contains("deploy.env:1"));
}

#[test]
fn actions_explain_tracks_expression_references() {
    let output = actions_command("explain")
        .args(["COMBINED", "-j", "build", "--show-values"])
        .assert()
        .success();
    output
        .stdout(predicate::str::contains("references:"))
        .stdout(predicate::str::contains("env.JOB_LEVEL"))
        .stdout(predicate::str::contains("matrix.os"))
        .stdout(predicate::str::contains("external context"));
}

#[test]
fn actions_explain_reports_runtime_github_env_writes() {
    let output = actions_command("scan").assert().success();
    output
        .stdout(predicate::str::contains("github-env-runtime"))
        .stdout(predicate::str::contains("RESULT"));
}

#[test]
fn actions_require_job_for_multi_job_workflow() {
    let output = actions_command("explain").arg("SHARED").assert().failure();
    output.stderr(predicate::str::contains("unknown job"));

    let output = actions_command("explain").arg("NOPE").assert().failure();
    output.stderr(predicate::str::contains("unknown job"));
}

#[test]
fn actions_json_output_redacts_and_keeps_expressions() {
    let output = actions_command("scan")
        .arg("--format")
        .arg("json")
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    let report: envorigin::actions::ActionsReport = serde_json::from_str(&stdout).unwrap();
    let build = report.jobs.iter().find(|job| job.id == "build").unwrap();
    assert_eq!(build.steps.len(), 4);
    let shared = build
        .steps
        .iter()
        .find(|step| step.name.as_deref() == Some("Configure"))
        .unwrap()
        .variables
        .iter()
        .find(|variable| variable.variable == "SHARED")
        .unwrap();
    assert!(shared
        .value
        .as_deref()
        .unwrap()
        .starts_with("<redacted sha256:"));
    let combined = build
        .variables
        .iter()
        .find(|variable| variable.variable == "COMBINED")
        .unwrap();
    assert!(combined
        .value
        .as_deref()
        .unwrap()
        .contains("${{ env.JOB_LEVEL }}"));
}

// --- raw env_file format ---

#[test]
fn raw_env_file_skips_interpolation_plain_env_file_does_not() {
    let raw_fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw/compose.yaml")
        .display()
        .to_string();
    let mut plain = Command::cargo_bin("envorigin").unwrap();
    plain
        .args([
            "explain",
            "PLAIN_REF",
            "-s",
            "web",
            "--show-values",
            "--no-docker-check",
            "--file",
        ])
        .arg(&raw_fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("\"from_dotenv\""));
    let mut raw = Command::cargo_bin("envorigin").unwrap();
    raw.args([
        "explain",
        "RAW_REF",
        "-s",
        "web",
        "--show-values",
        "--no-docker-check",
        "--file",
    ])
    .arg(&raw_fixture)
    .assert()
    .success()
    .stdout(predicate::str::contains("\"${BASE}\""))
    .stdout(predicate::str::contains("from_dotenv").not());
}

// --- COMPOSE_ENV_FILES ---

fn env_files_fixture() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/env-files/compose.yaml")
        .display()
        .to_string()
}

#[test]
fn compose_env_files_expands_and_later_files_win() {
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .env("COMPOSE_ENV_FILES", "env/a.env:env/b.env")
        .args(["scan", "--show-values", "--no-docker-check", "--file"])
        .arg(env_files_fixture())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"from_a\""))
        .stdout(predicate::str::contains("\"from_b\""))
        .stdout(predicate::str::contains("\"b\""));
}

#[test]
fn compose_env_files_missing_entry_warns_and_continues() {
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .env("COMPOSE_ENV_FILES", "env/a.env:does-not-exist.env")
        .args(["scan", "--no-docker-check", "--file"])
        .arg(env_files_fixture())
        .assert()
        .success()
        .stdout(predicate::str::contains("compose-env-files-missing"))
        .stdout(predicate::str::contains("\"from_a\"").not())
        .stdout(predicate::str::contains("does-not-exist.env"));
}

// --- shadowing diagnostics ---

#[test]
fn scan_marks_shadowed_dead_code_lines() {
    let output = command("scan", "precedence").assert().success();
    output
        .stdout(predicate::str::contains("is shadowed by"))
        .stdout(predicate::str::contains("env/web.env:1"));
}

#[test]
fn scan_omits_shadow_notes_when_nothing_is_shadowed() {
    let output = command("scan", "basic").assert().success();
    output.stdout(predicate::str::contains("is shadowed by").not());
}

// --- workflow inputs ---

#[test]
fn actions_explain_resolves_declared_inputs() {
    let inputs_fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/actions-inputs/.github/workflows/inputs.yaml")
        .display()
        .to_string();
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .args([
            "actions",
            "explain",
            "TARGET_ENV",
            "-j",
            "deploy",
            "--show-values",
            "--file",
        ])
        .arg(&inputs_fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("inputs.environment"))
        .stdout(predicate::str::contains("workflow input"))
        .stdout(predicate::str::contains("inputs.yaml:5"));
    let mut scan = Command::cargo_bin("envorigin").unwrap();
    scan.args(["actions", "scan", "--file"])
        .arg(&inputs_fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("workflow inputs:"))
        .stdout(predicate::str::contains("dry_run"));
}

// --- audit ---

fn audit_fixture() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audit/compose.yaml")
        .display()
        .to_string()
}

fn audit_command() -> Command {
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .arg("audit")
        .arg("--no-docker-check")
        .arg("--file")
        .arg(audit_fixture());
    command
}

#[test]
fn audit_reports_sensitive_shadowed_and_unused() {
    let output = audit_command().assert().failure();
    output
        .stdout(predicate::str::contains("2 error(s), 3 warning(s), 1 info"))
        .stdout(predicate::str::contains("sensitive-value"))
        .stdout(predicate::str::contains("sensitive-placeholder"))
        .stdout(predicate::str::contains("API_TOKEN"))
        .stdout(predicate::str::contains("DB_PASSWORD"))
        .stdout(predicate::str::contains("shadowed-env-line"))
        .stdout(predicate::str::contains("unused-interpolation-variable"))
        .stdout(predicate::str::contains("ORPHAN"));
}

#[test]
fn audit_fail_on_none_succeeds_even_with_errors() {
    let output = audit_command()
        .arg("--fail-on")
        .arg("none")
        .assert()
        .success();
    output
        .stdout(predicate::str::contains("sensitive-value"))
        .stdout(predicate::str::contains("sensitive-placeholder"));
}

#[test]
fn audit_fail_on_warning_fails_on_warnings() {
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .args([
            "audit",
            "--fail-on",
            "warning",
            "--no-docker-check",
            "--file",
        ])
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/basic/compose.yaml")
                .display()
                .to_string(),
        )
        .assert()
        .success();
}

#[test]
fn audit_clean_compose_exits_zero() {
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .args(["audit", "--no-docker-check", "--file"])
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/basic/compose.yaml")
                .display()
                .to_string(),
        )
        .assert()
        .success()
        .stdout(predicate::str::contains("0 error(s), 0 warning(s), 0 info"));
}

#[test]
fn actions_audit_deduplicates_across_scopes() {
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .args(["actions", "audit", "--file"])
        .arg(workflow_path())
        .assert()
        .success()
        .stdout(predicate::str::contains("2 warning(s)"))
        .stdout(predicate::str::contains("shadowed-env-line"));
}

// --- diff ---

fn diff_fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/diff")
        .join(name)
        .display()
        .to_string()
}

#[test]
fn diff_reports_drift_and_exclusive_keys() {
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .arg("diff")
        .arg(diff_fixture("local.env"))
        .arg(diff_fixture("ci.env"))
        .assert()
        .success()
        .stdout(predicate::str::contains("drift (2):"))
        .stdout(predicate::str::contains("DB_HOST"))
        .stdout(predicate::str::contains("only in local.env"))
        .stdout(predicate::str::contains("ONLY_LOCAL"))
        .stdout(predicate::str::contains("ONLY_CI"));
}

#[test]
fn diff_redacts_sensitive_values_by_default() {
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .arg("diff")
        .arg(diff_fixture("local.env"))
        .arg(diff_fixture("ci.env"))
        .assert()
        .success()
        .stdout(predicate::str::contains("<redacted sha256:"))
        .stdout(predicate::str::contains("secret-local").not());
}

#[test]
fn diff_show_values_reveals_sensitive() {
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .arg("diff")
        .arg("--show-values")
        .arg(diff_fixture("local.env"))
        .arg(diff_fixture("ci.env"))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"secret-local\""));
}

// --- graph ---

#[test]
fn graph_renders_provenance_edges() {
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .arg("graph")
        .arg("--no-docker-check")
        .arg("--file")
        .arg(fixture("precedence"))
        .assert()
        .success()
        .stdout(predicate::str::contains("graph LR"))
        .stdout(predicate::str::contains("subgraph svc_web"))
        .stdout(predicate::str::contains("-->|\"ServiceEnvironment\"|"))
        .stdout(predicate::str::contains("-.->|\"shadowed\"|"))
        .stdout(predicate::str::contains("-.->|\"derived\"|"));
}

#[test]
fn actions_graph_renders_job_step_and_sources() {
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .args(["actions", "graph", "--file"])
        .arg(workflow_path())
        .assert()
        .success()
        .stdout(predicate::str::contains("subgraph job_build"))
        .stdout(predicate::str::contains("step_"))
        .stdout(predicate::str::contains("-->|\"StepEnv\"|"));
}

// --- GitLab CI ---

fn gitlab_fixture() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/gitlab/.gitlab-ci.yml")
        .display()
        .to_string()
}

fn gitlab_command(subcommand: &str) -> Command {
    let mut command = Command::cargo_bin("envorigin").unwrap();
    command
        .arg("gitlab")
        .arg(subcommand)
        .arg("--file")
        .arg(gitlab_fixture());
    command
}

#[test]
fn gitlab_scan_lists_globals_jobs_and_includes() {
    let output = gitlab_command("scan").assert().success();
    output
        .stdout(predicate::str::contains("GitLab CI analysis"))
        .stdout(predicate::str::contains("global variables:"))
        .stdout(predicate::str::contains("job build"))
        .stdout(predicate::str::contains("include files:"))
        .stdout(predicate::str::contains("gitlab-include-external"));
}

#[test]
fn gitlab_explain_tracks_interpolation_references() {
    let output = gitlab_command("explain")
        .args(["BUILD_ID", "-j", "build", "--show-values"])
        .assert()
        .success();
    output
        .stdout(predicate::str::contains("references:"))
        .stdout(predicate::str::contains("CI_PIPELINE_ID"))
        .stdout(predicate::str::contains("GitLab predefined"))
        .stdout(predicate::str::contains("APP_NAME"));
}

#[test]
fn gitlab_explain_shows_include_shadowing_chain() {
    let output = gitlab_command("explain")
        .args(["APP_NAME", "--show-values"])
        .assert()
        .success();
    output
        .stdout(predicate::str::contains("winner: global variables"))
        .stdout(predicate::str::contains("[shadowed] include file"))
        .stdout(predicate::str::contains("include_app_name"));
}

#[test]
fn gitlab_explain_job_override_wins() {
    let output = gitlab_command("explain")
        .args(["GLOBAL_ONLY", "-j", "build", "--show-values"])
        .assert()
        .success();
    output
        .stdout(predicate::str::contains("winner: job variables"))
        .stdout(predicate::str::contains("overridden_in_job"))
        .stdout(predicate::str::contains("[shadowed] global variables"));
}
