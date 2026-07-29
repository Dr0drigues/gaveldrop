//! The facade, exercised as a user would.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "panicking is how a test reports failure; an integration test file is its own \
              crate, so the library's cfg_attr does not cover it"
)]

use std::fs;
use std::process::{Command, Output};

use assert_cmd::cargo::cargo_bin;

/// Fails with something actionable when the fake binary has not been built.
///
/// `cargo test -p gaveldrop-cli` builds this crate's dependencies as libraries, not the
/// `gaveldrop-fake` **binary**. `cargo test --workspace` — which is the test gate — builds
/// everything. Without this check the symptom would be every case failing for an unrelated
/// reason.
fn require_fake_binary() {
    let fake = cargo_bin("gaveldrop-fake");
    assert!(
        fake.is_file(),
        "{} is missing. Run `cargo test --workspace` (or `mise run test`) rather than \
         testing this crate alone: the fake binary is built by its own crate, not by this \
         one's dependency graph.",
        fake.display()
    );
}

fn project(case_yaml: &str) -> tempfile::TempDir {
    require_fake_binary();

    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("tests/cases")).unwrap();
    fs::write(
        dir.path().join("gaveldrop.yaml"),
        "cases: tests/cases/**/*.yaml\nfake:\n  bins: [git]\n",
    )
    .unwrap();
    fs::write(dir.path().join("tests/cases/one.yaml"), case_yaml).unwrap();
    dir
}

fn run(dir: &tempfile::TempDir) -> Output {
    Command::new(cargo_bin("gaveldrop"))
        .current_dir(dir.path())
        .output()
        .unwrap()
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_passing_case_exits_zero_and_says_so() {
    let dir = project(
        "name: it-works\nweight: 5\nsetup:\n  run: [\"sh\", \"-c\", \"echo hello\"]\nexpect:\n  exit_code: 0\n  stdout:\n    contains: [\"hello\"]\n",
    );
    let output = run(&dir);
    let rendered = stdout_of(&output);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_of(&output)
    );
    assert!(rendered.contains("it-works"), "output was:\n{rendered}");
    assert!(rendered.contains("5/5"), "output was:\n{rendered}");
}

#[test]
fn a_failing_case_exits_non_zero_and_names_what_broke() {
    let dir = project(
        "name: it-breaks\nweight: 8\nsetup:\n  run: [\"sh\", \"-c\", \"exit 3\"]\nexpect:\n  exit_code: 0\n",
    );
    let output = run(&dir);
    let rendered = stdout_of(&output);

    assert_ne!(output.status.code(), Some(0));
    for fragment in ["it-breaks", "expect.exit_code", "3"] {
        assert!(
            rendered.contains(fragment),
            "missing {fragment:?} in:\n{rendered}"
        );
    }
}

#[test]
fn a_faked_binary_answers_instead_of_the_real_one() {
    let dir = project(
        r#"
name: git-is-faked
weight: 5
setup:
  run: ["sh", "-c", "git status --porcelain"]
fake:
  rules:
    - match: { bin: git, args_contain: "status --porcelain" }
      stdout: " M src/index.js"
    - match: {}
      exit: 127
expect:
  exit_code: 0
  stdout:
    contains: ["M src/index.js"]
  calls:
    git: 1
"#,
    );
    let output = run(&dir);

    assert_eq!(
        output.status.code(),
        Some(0),
        "the fake must answer in place of the real git, inside a repository that is not \
         even a git checkout.\nstdout:\n{}\nstderr:\n{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

#[test]
fn an_unexpected_call_fails_the_case_loudly() {
    let dir = project(
        r#"
name: nothing-was-declared
weight: 5
setup:
  run: ["sh", "-c", "git status || true"]
fake:
  rules:
    - match: { bin: git, args_contain: "push" }
      stdout: "pushed"
    - match: {}
      exit: 127
      stderr: "unexpected call"
expect:
  exit_code: 0
"#,
    );
    let output = run(&dir);
    let rendered = stdout_of(&output);

    assert_ne!(output.status.code(), Some(0));
    assert!(
        rendered.contains("unexpected calls") && rendered.contains("git"),
        "an unexpected call must fail the case whether or not it mentions calls:\n{rendered}"
    );
}

#[test]
fn a_missing_config_says_what_to_create() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(cargo_bin("gaveldrop"))
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert_ne!(output.status.code(), Some(0));
    assert!(
        stderr_of(&output).contains("gaveldrop.yaml"),
        "stderr was: {}",
        stderr_of(&output)
    );
}

#[test]
fn a_pattern_matching_no_case_is_an_error_rather_than_a_green_run() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("gaveldrop.yaml"),
        "cases: tests/cases/**/*.yaml\n",
    )
    .unwrap();

    let output = Command::new(cargo_bin("gaveldrop"))
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert_ne!(
        output.status.code(),
        Some(0),
        "a suite with no cases would pass while proving nothing"
    );
    assert!(stderr_of(&output).contains("tests/cases"));
}

#[test]
fn the_config_path_and_root_can_be_pointed_elsewhere() {
    let dir = project(
        "name: it-works\nweight: 2\nsetup:\n  run: [\"sh\", \"-c\", \"true\"]\nexpect: { exit_code: 0 }\n",
    );
    let elsewhere = tempfile::tempdir().unwrap();

    let output = Command::new(cargo_bin("gaveldrop"))
        .current_dir(elsewhere.path())
        .arg("--config")
        .arg(dir.path().join("gaveldrop.yaml"))
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "the cases pattern must resolve from the config's directory, not from wherever the \
         command happened to be run.\nstderr:\n{}",
        stderr_of(&output)
    );
}
