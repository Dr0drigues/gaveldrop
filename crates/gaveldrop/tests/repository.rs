//! Checks about this repository rather than about the crate.
//!
//! Excluded from the published package, like `own_cases.rs`: these read files two levels above the
//! manifest, which do not exist in an extracted crate. A test that cannot pass where it is shipped
//! is not a test.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "panicking is how a test reports failure; an integration test file is its own crate, \
              so the library's cfg_attr does not cover it"
)]

use std::path::PathBuf;

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(relative: &str) -> String {
    let path = repository_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
}

/// The commands `.mise.toml` calls the three gates.
///
/// Read out of the file rather than hardcoded here, which would move the drift instead of catching
/// it: a third copy is not an improvement on two.
fn gates_from_mise(mise: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut task = None;

    for line in mise.lines() {
        let line = line.trim();
        if let Some(name) = line
            .strip_prefix("[tasks.")
            .and_then(|r| r.strip_suffix(']'))
        {
            task = Some(name.to_string());
        }
        if let Some(rest) = line.strip_prefix("run = \"")
            && let Some(command) = rest.strip_suffix('"')
            && matches!(task.as_deref(), Some("fmt" | "lint" | "test"))
        {
            commands.push(command.to_string());
        }
    }

    commands
}

/// The `cargo` commands the Linux gate job runs, in order.
///
/// Only that job. Other jobs repeat the test command or install things, so widening this would make
/// the check refuse legitimate additions.
///
/// The end is "the next job", found by indentation, rather than the name of whichever job happens to
/// follow. The first version stopped at `macos:` and kept working only because the job inserted
/// before it ran no `cargo` command — a check that passes by luck is the kind that fails later for a
/// reason nobody connects to this.
fn gates_from_ci(ci: &str) -> Vec<String> {
    ci.lines()
        .skip_while(|line| !line.contains("The three gates (Linux)"))
        .skip(1)
        .take_while(|line| !starts_a_job(line))
        .filter_map(|line| line.trim().strip_prefix("run: cargo "))
        .map(|rest| format!("cargo {rest}"))
        .collect()
}

/// True for a line like `  macos:` — a key of the `jobs:` mapping, and nothing deeper.
fn starts_a_job(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("  ") else {
        return false;
    };
    !rest.starts_with(' ')
        && !rest.starts_with('#')
        && rest.ends_with(':')
        && rest
            .trim_end_matches(':')
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[test]
fn the_three_gates_are_the_same_locally_and_in_ci() {
    let mise = gates_from_mise(&read(".mise.toml"));
    let ci = gates_from_ci(&read(".github/workflows/ci.yml"));

    assert_eq!(
        mise.len(),
        3,
        "three gates were expected in `.mise.toml` and {} were found: {mise:?}. If a fourth is \
         legitimate, this test is where it gets acknowledged",
        mise.len()
    );

    for gate in &mise {
        assert!(
            ci.contains(gate),
            "`{gate}` is a gate locally and CI does not run it. That direction is the dangerous \
             one: a rule tightened in `.mise.toml` and forgotten in CI makes CI the laxer of the \
             two, so a commit passes review and fails on the next contributor's machine.\n  \
             local: {mise:?}\n  ci:    {ci:?}"
        );
    }

    for command in &ci {
        assert!(
            mise.contains(command),
            "CI runs `{command}` in the gate job and no `mise` task does. Less dangerous than the \
             other direction, and still worth refusing: it means a contributor cannot reproduce \
             the gate before pushing.\n  local: {mise:?}\n  ci:    {ci:?}"
        );
    }
}

#[test]
fn the_extraction_notices_a_command_that_drifted() {
    let mise = "[tasks.fmt]\nrun = \"cargo fmt --all -- --check\"\n[tasks.lint]\nrun = \"cargo clippy --all-targets -- -D warnings\"\n[tasks.test]\nrun = \"cargo test --workspace\"\n";
    // A job between the gates and the end, running a `cargo` command of its own: the extraction has
    // to stop before it, or an unrelated job would be read as a gate.
    let drifted = "  gates:\n    name: The three gates (Linux)\n    steps:\n      - name: Format\n        run: cargo fmt --check\n  other:\n    steps:\n      - run: cargo install something\n  macos:\n";

    let local = gates_from_mise(mise);
    let ci = gates_from_ci(drifted);

    assert_eq!(local.len(), 3, "the reader must find all three: {local:?}");
    assert_eq!(
        ci,
        vec!["cargo fmt --check"],
        "and only the gate job's commands, stopping at the next job: {ci:?}"
    );
    assert!(
        !ci.contains(&local[0]),
        "`cargo fmt --check` is not `cargo fmt --all -- --check` — it checks one crate instead of \
         the workspace. Without this the comparison above could hold while the two had drifted, \
         which is the failure this whole test exists to prevent"
    );
}

#[test]
fn a_mise_task_that_is_not_a_gate_is_ignored() {
    let mise = read(".mise.toml");

    let gates = gates_from_mise(&mise);
    assert!(
        mise.contains("[tasks.commits]"),
        "this test is vacant unless `.mise.toml` really does hold a non-gate task"
    );
    assert!(
        !gates.iter().any(|gate| gate.contains("committed")),
        "`commits`, `changelog` and `release` are conveniences rather than gates, and CI has no \
         reason to mirror them: {gates:?}"
    );
}
