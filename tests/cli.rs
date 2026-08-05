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
