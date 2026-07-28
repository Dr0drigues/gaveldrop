//! The binary, tested end to end: symlink it under a binary's name, invoke it, look at
//! what it did.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "panicking is how a test reports failure; an integration test file is its \
              own crate, so lib.rs's cfg_attr does not cover it"
)]

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use assert_cmd::cargo::cargo_bin;

struct Sandbox {
    _dir: tempfile::TempDir,
    root: PathBuf,
    bin_dir: PathBuf,
    journal: PathBuf,
    state: PathBuf,
    scenario: PathBuf,
}

fn sandbox(faked_name: &str, scenario_yaml: &str) -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    std::os::unix::fs::symlink(cargo_bin("gaveldrop-fake"), bin_dir.join(faked_name)).unwrap();

    let scenario = root.join("scenario.yaml");
    fs::write(&scenario, scenario_yaml).unwrap();

    Sandbox {
        _dir: dir,
        journal: root.join("journal.jsonl"),
        state: root.join("state"),
        scenario,
        bin_dir,
        root,
    }
}

impl Sandbox {
    fn command(&self, faked_name: &str) -> Command {
        let mut command = Command::new(self.bin_dir.join(faked_name));
        command
            .current_dir(&self.root)
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin_dir.display()))
            .env("GAVELDROP_SCENARIO", &self.scenario)
            .env("GAVELDROP_STATE", &self.state)
            .env("GAVELDROP_JOURNAL", &self.journal)
            .env("GAVELDROP_DIR", &self.root)
            .env("GAVELDROP_CASE", "test");
        command
    }

    fn invoke(&self, faked_name: &str, args: &[&str]) -> std::process::Output {
        self.command(faked_name).args(args).output().unwrap()
    }

    fn link(&self, faked_name: &str) {
        std::os::unix::fs::symlink(cargo_bin("gaveldrop-fake"), self.bin_dir.join(faked_name))
            .unwrap();
    }

    fn executable(&self, relative: &str, body: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn journal(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(&self.journal)
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

#[test]
fn static_mode_writes_and_exits_with_the_requested_code() {
    let sandbox = sandbox(
        "kubectl",
        r#"
rules:
  - match: { args_contain: "get pods" }
    stdout: "pod-a  Running  1/1"
  - match: {}
    exit: 127
    stderr: "unexpected call"
"#,
    );

    let output = sandbox.invoke("kubectl", &["get", "pods"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout_of(&output), "pod-a  Running  1/1");
}

#[test]
fn the_catch_all_answers_and_flags_itself_in_the_journal() {
    let sandbox = sandbox(
        "kubectl",
        r#"
rules:
  - match: { args_contain: "get pods" }
    stdout: "ok"
  - match: {}
    exit: 127
    stderr: "unexpected call"
"#,
    );

    let output = sandbox.invoke("kubectl", &["delete", "ns", "prod"]);
    assert_eq!(output.status.code(), Some(127));
    assert!(stderr_of(&output).contains("unexpected call"));

    let journal = sandbox.journal();
    assert_eq!(journal.len(), 1);
    assert_eq!(
        journal[0]["catch_all"], true,
        "an unexpected call must be identifiable in the journal"
    );
    assert_eq!(journal[0]["bin"], "kubectl");
    assert_eq!(
        journal[0]["args"],
        serde_json::json!(["delete", "ns", "prod"])
    );
}

#[test]
fn the_counter_varies_the_response_by_rank() {
    let sandbox = sandbox(
        "git",
        r#"
rules:
  - match: { call: 1 }
    stdout: "first"
  - match: { call: 2 }
    exit: 1
    stderr: "fatal: not a repo"
  - match: {}
    stdout: "later"
"#,
    );

    assert_eq!(stdout_of(&sandbox.invoke("git", &["status"])), "first");

    let second = sandbox.invoke("git", &["status"]);
    assert_eq!(second.status.code(), Some(1));
    assert!(stderr_of(&second).contains("not a repo"));

    assert_eq!(stdout_of(&sandbox.invoke("git", &["status"])), "later");

    let journal = sandbox.journal();
    assert_eq!(journal.len(), 3);
    assert_eq!(journal[0]["call"], 1);
    assert_eq!(journal[2]["call"], 3);
}

#[test]
fn each_faked_binary_has_its_own_counter() {
    let sandbox = sandbox(
        "git",
        "rules:\n  - match: { call: 1 }\n    stdout: \"first\"\n  - match: {}\n    stdout: \"later\"\n",
    );
    sandbox.link("gh");

    assert_eq!(stdout_of(&sandbox.invoke("git", &[])), "first");
    assert_eq!(
        stdout_of(&sandbox.invoke("gh", &[])),
        "first",
        "`gh` is on its own first call: its counter is independent of `git`'s"
    );
    assert_eq!(stdout_of(&sandbox.invoke("git", &[])), "later");
}

#[test]
fn stdin_contains_discriminates_on_standard_input() {
    let sandbox = sandbox(
        "claude",
        r#"
rules:
  - match: { stdin_contains: "AGENT: alpha" }
    stdout: "alpha answer"
  - match: {}
    stdout: "default answer"
"#,
    );

    let mut child = sandbox
        .command("claude")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"noise\nAGENT: alpha\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert_eq!(stdout_of(&output), "alpha answer");
}

#[test]
fn exec_real_calls_the_real_binary_and_journals_it() {
    let sandbox = sandbox("mock-tool", "rules:\n  - match: {}\n    exec: real\n");

    let real_dir = sandbox.root.join("real-bin");
    fs::create_dir_all(&real_dir).unwrap();
    let real = real_dir.join("mock-tool");
    fs::write(&real, "#!/bin/sh\necho \"the real one: $*\"\n").unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o755)).unwrap();

    let output = sandbox
        .command("mock-tool")
        .arg("hello")
        .env(
            "PATH",
            format!(
                "{}:{}:/usr/bin:/bin",
                sandbox.bin_dir.display(),
                real_dir.display()
            ),
        )
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert!(
        stdout_of(&output).contains("the real one: hello"),
        "output was: {}",
        stdout_of(&output)
    );
    assert_eq!(
        sandbox.journal()[0]["passthrough"],
        true,
        "a passthrough is journaled all the same"
    );
}

#[test]
fn exec_script_delegates_to_the_project_executable() {
    let sandbox = sandbox(
        "kubectl",
        r#"
rules:
  - match: { args_contain: "get -o json" }
    exec: ./pods.sh
  - match: {}
    exit: 127
"#,
    );
    sandbox.executable("pods.sh", "#!/bin/sh\necho '{\"items\":[]}'\n");

    let output = sandbox.invoke("kubectl", &["get", "-o", "json"]);
    assert_eq!(stdout_of(&output), "{\"items\":[]}");
}

#[test]
fn render_shapes_the_response_with_a_script() {
    let sandbox = sandbox(
        "claude",
        "render: ./shape.sh\nrules:\n  - match: {}\n    stdout: \"I suggest a cache\"\n",
    );
    sandbox.executable(
        "shape.sh",
        "#!/bin/sh\ncat > payload.json\necho '{\"type\":\"assistant\"}'\n",
    );

    let output = sandbox.invoke("claude", &[]);

    assert_eq!(
        stdout_of(&output),
        r#"{"type":"assistant"}"#,
        "the script's output must reach the caller, not the rule's"
    );

    let payload: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(sandbox.root.join("payload.json")).unwrap())
            .unwrap();
    let contract = "the payload must carry the announced contract";
    assert_eq!(payload["rule"]["stdout"], "I suggest a cache", "{contract}");
    assert_eq!(payload["invocation"]["bin"], "claude", "{contract}");
    assert_eq!(payload["call"], 1, "{contract}");
}

#[test]
fn a_rule_exit_survives_a_render_hook() {
    let sandbox = sandbox(
        "claude",
        "render: ./shape.sh\nrules:\n  - match: {}\n    stdout: \"nope\"\n    exit: 1\n",
    );
    sandbox.executable("shape.sh", "#!/bin/sh\ncat > /dev/null\necho shaped\n");

    let output = sandbox.invoke("claude", &[]);
    assert_eq!(stdout_of(&output), "shaped");
    assert_eq!(
        output.status.code(),
        Some(1),
        "the hook shapes bytes; the rule still decides whether the faked tool failed"
    );
}

#[test]
fn a_render_hook_that_fails_is_reported_as_a_harness_failure() {
    let sandbox = sandbox(
        "claude",
        "render: ./broken.sh\nrules:\n  - match: {}\n    stdout: \"ok\"\n",
    );
    sandbox.executable("broken.sh", "#!/bin/sh\ncat > /dev/null\nexit 3\n");

    let output = sandbox.invoke("claude", &[]);
    assert_eq!(
        output.status.code(),
        Some(125),
        "a broken hook is our failure, never passed off as the faked tool's exit code"
    );
    assert!(stderr_of(&output).contains("render hook"));
}

#[test]
fn a_scenario_without_a_catch_all_fails_with_the_harness_failure_code() {
    let sandbox = sandbox("git", "rules:\n  - match: { bin: git }\n    stdout: ok\n");

    let output = sandbox.invoke("git", &[]);
    assert_eq!(
        output.status.code(),
        Some(125),
        "125 is reserved for the fake's own failures, so it stays distinguishable from \
         a simulated exit code"
    );
    assert!(stderr_of(&output).contains("catch-all"));
}

#[test]
fn a_missing_scenario_fails_with_the_harness_failure_code() {
    let sandbox = sandbox("git", "rules:\n  - match: {}\n    stdout: ok\n");
    fs::remove_file(&sandbox.scenario).unwrap();

    let output = sandbox.invoke("git", &[]);
    assert_eq!(output.status.code(), Some(125));
    assert!(stderr_of(&output).contains("scenario.yaml"));
}

#[test]
fn the_journal_loses_nothing_under_concurrent_calls() {
    let sandbox = sandbox("git", "rules:\n  - match: {}\n    stdout: ok\n");

    let children: Vec<_> = (0..16)
        .map(|index| {
            sandbox
                .command("git")
                .arg(format!("call-{index}"))
                .stdout(Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for mut child in children {
        child.wait().unwrap();
    }

    assert_eq!(
        sandbox.journal().len(),
        16,
        "an append-only journal must lose no line under concurrent writes"
    );
}

#[test]
fn exec_real_without_a_real_binary_fails_cleanly_instead_of_looping() {
    let sandbox = sandbox("nowhere-tool", "rules:\n  - match: {}\n    exec: real\n");

    let output = sandbox
        .command("nowhere-tool")
        .env("PATH", sandbox.bin_dir.display().to_string())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(125),
        "had PATH lookup not skipped our own directory, this would recurse until the \
         process table gave out"
    );
    assert!(stderr_of(&output).contains("nowhere on PATH"));
}

#[test]
fn bin_is_the_symlink_name_not_the_real_program_name() {
    let sandbox = sandbox(
        "kubectl",
        "rules:\n  - match: { bin: kubectl }\n    stdout: \"seen as kubectl\"\n  - match: {}\n    exit: 3\n",
    );

    let output = sandbox.invoke("kubectl", &[]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout_of(&output), "seen as kubectl");
    assert_eq!(sandbox.journal()[0]["bin"], "kubectl");
}
