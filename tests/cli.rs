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
