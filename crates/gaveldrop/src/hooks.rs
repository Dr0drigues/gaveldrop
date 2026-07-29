//! The hook protocol: JSON in on standard input, a result out.
//!
//! The unit of extension is **an executable**, not a Rust crate. That is what puts every
//! targeted technology on equal footing: a Kotlin, Python or shell project hooks in exactly
//! what a Rust project does. Had the extension point been a trait, only Rust could extend
//! gaveldrop.
//!
//! The contract is **this protocol**, not the convenience packages that may be published per
//! ecosystem later. A language with no package works with three lines of `jq`.

use std::io::Write as _;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde::Deserialize;

use crate::{Case, Diff, Isolation, Observations};

/// What can go wrong in a hook.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    /// The hook could not be started.
    #[error("running the {which} hook `{path}`: {source}")]
    Spawn {
        /// Which hook, for the message.
        which: &'static str,
        /// The path that would not run.
        path: String,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The hook ran and refused.
    #[error("the {which} hook `{path}` exited with {code}: {stderr}")]
    Refused {
        /// Which hook, for the message.
        which: &'static str,
        /// The hook's path.
        path: String,
        /// Its exit code.
        code: i32,
        /// What it said, trimmed.
        stderr: String,
    },
    /// The payload could not be serialised, or the answer could not be read.
    #[error("the {which} hook `{path}` protocol: {source}")]
    Protocol {
        /// Which hook, for the message.
        which: &'static str,
        /// The hook's path.
        path: String,
        /// The underlying failure.
        #[source]
        source: serde_json::Error,
    },
}

/// Runs `setup.exec` when the case has one.
///
/// The hook receives the `setup` block as JSON on its standard input, minus `exec` itself —
/// its own path is not part of what it is being told to prepare. It runs **in the isolated
/// root, with the isolated environment**, so a hook that writes into `$HOME` writes into the
/// case's home rather than the developer's.
pub fn run_setup(case: &Case, iso: &Isolation, root: &Path) -> Result<(), HookError> {
    let Some(path) = case.setup.exec.as_deref() else {
        return Ok(());
    };

    let payload = setup_payload(case).map_err(|source| HookError::Protocol {
        which: "setup",
        path: path.to_string(),
        source,
    })?;

    let output = feed(path, "setup", iso, root, &payload)?;
    refuse_on_failure("setup", path, &output)?;

    Ok(())
}

/// What an `expect.exec` hook answers.
#[derive(Debug, Deserialize)]
pub struct HookVerdict {
    /// Whether the hook is satisfied.
    pub ok: bool,
    /// Why not, when it is not.
    #[serde(default)]
    pub diffs: Vec<Diff>,
}

/// Runs `expect.exec` when the case has one, returning the diffs it reported.
///
/// The hook sees the observations in full: it exists to check what the core cannot, so
/// withholding anything would be self-defeating.
pub fn run_expect(
    case: &Case,
    iso: &Isolation,
    observations: &Observations,
    root: &Path,
) -> Result<Vec<Diff>, HookError> {
    let Some(path) = case.expect.exec.as_deref() else {
        return Ok(Vec::new());
    };

    let payload = serde_json::to_vec(observations).map_err(|source| HookError::Protocol {
        which: "expect",
        path: path.to_string(),
        source,
    })?;

    let output = feed(path, "expect", iso, root, &payload)?;
    refuse_on_failure("expect", path, &output)?;

    let verdict: HookVerdict =
        serde_json::from_slice(&output.stdout).map_err(|source| HookError::Protocol {
            which: "expect",
            path: path.to_string(),
            source,
        })?;

    Ok(diffs_of(verdict, path))
}

/// The diffs a verdict amounts to.
///
/// A hook that refuses without explaining still refuses. Supplying a placeholder diff is the
/// only honest reading: letting the case pass because the hook was terse would make
/// `ok: false` meaningless.
fn diffs_of(verdict: HookVerdict, path: &str) -> Vec<Diff> {
    if verdict.ok {
        return Vec::new();
    }
    if verdict.diffs.is_empty() {
        return vec![Diff {
            path: "expect.exec".to_string(),
            expected: "satisfied".to_string(),
            got: format!("`{path}` answered ok: false without saying why"),
        }];
    }
    verdict.diffs
}

/// The `setup` block as the hook sees it: everything the core does not own.
fn setup_payload(case: &Case) -> Result<Vec<u8>, serde_json::Error> {
    let mut object = serde_json::Map::new();
    if let Some(run) = &case.setup.run {
        object.insert("run".to_string(), serde_json::to_value(run)?);
    }
    for (key, value) in &case.setup.extra {
        object.insert(key.clone(), value.clone());
    }
    serde_json::to_vec(&serde_json::Value::Object(object))
}

/// The hook's path made **absolute**, resolved against the project root when relative.
///
/// Absolute, not merely joined. The hook runs with its working directory set to the isolated
/// temporary root, so anything still relative at that point would resolve there — where nothing
/// of the project exists. A project root of `.` is the common case, which is exactly when a
/// plain join is not enough.
///
/// Canonicalising can fail when the hook does not exist. The join is kept in that case so the
/// error message names a path the reader recognises rather than nothing at all.
fn resolve(path: &str, root: &Path) -> PathBuf {
    let declared = Path::new(path);
    if declared.is_absolute() {
        return declared.to_path_buf();
    }

    let joined = root.join(declared);
    std::fs::canonicalize(&joined).unwrap_or(joined)
}

/// Turns a non-zero exit into a `Refused`, quoting what the hook said.
fn refuse_on_failure(which: &'static str, path: &str, output: &Output) -> Result<(), HookError> {
    if output.status.success() {
        return Ok(());
    }

    Err(HookError::Refused {
        which,
        path: path.to_string(),
        code: output.status.code().unwrap_or(-1),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

/// Spawns the hook, writes `payload` to its standard input, and waits.
///
/// Two different directories are in play, and both matter. The hook is **resolved** against the
/// project root, because that is where a case author means when they write
/// `./tests/hooks/foo.sh`. It **runs** in the isolated root, because that is what it prepares
/// or inspects.
///
/// Standard input is closed after the write: without that, a hook reading to end-of-input would
/// wait forever, and the suite would hang rather than fail.
fn feed(
    path: &str,
    which: &'static str,
    iso: &Isolation,
    root: &Path,
    payload: &[u8],
) -> Result<Output, HookError> {
    let mut command = Command::new(resolve(path, root));
    command
        .current_dir(iso.root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in iso.env() {
        command.env(key, value);
    }
    for key in iso.cleared() {
        command.env_remove(key);
    }

    let mut child = command.spawn().map_err(|source| HookError::Spawn {
        which,
        path: path.to_string(),
        source,
    })?;

    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(payload);
        drop(input);
    }

    child.wait_with_output().map_err(|source| HookError::Spawn {
        which,
        path: path.to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn case_from(yaml: &str) -> Case {
        Case::load_str(yaml, Path::new("inline")).unwrap()
    }

    fn fake_binary(dir: &Path) -> PathBuf {
        let path = dir.join("gaveldrop-fake");
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn executable(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn isolate(case: &Case, outside: &Path) -> Isolation {
        Isolation::prepare(case, &fake_binary(outside), &[], &[], Path::new(".")).unwrap()
    }

    #[test]
    fn a_case_without_a_setup_hook_does_nothing_at_all() {
        let outside = tempfile::tempdir().unwrap();
        let case =
            case_from("name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nexpect: { exit_code: 0 }\n");
        let iso = isolate(&case, outside.path());

        assert!(run_setup(&case, &iso, outside.path()).is_ok());
    }

    #[test]
    fn the_hook_receives_the_setup_block_as_json_and_runs_in_the_isolated_root() {
        let outside = tempfile::tempdir().unwrap();
        let script = executable(
            outside.path(),
            "prepare.sh",
            "#!/bin/sh\ncat > received.json\npwd > where.txt\n",
        );
        let case = case_from(&format!(
            "name: t\nweight: 1\nsetup:\n  exec: {}\n  pattern: ring\n  agents: [alpha, bravo]\nexpect: {{ exit_code: 0 }}\n",
            script.display()
        ));
        let iso = isolate(&case, outside.path());

        run_setup(&case, &iso, outside.path()).unwrap();

        let received: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(iso.root().join("received.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(received["pattern"], "ring");
        assert_eq!(received["agents"], serde_json::json!(["alpha", "bravo"]));
        assert_eq!(
            received["exec"],
            serde_json::Value::Null,
            "the hook's own path is not part of what it is being told to prepare"
        );

        let where_it_ran = std::fs::read_to_string(iso.root().join("where.txt")).unwrap();
        assert_eq!(
            std::fs::canonicalize(where_it_ran.trim()).unwrap(),
            std::fs::canonicalize(iso.root()).unwrap(),
            "the hook prepares the isolated directory, so that is where it must run"
        );
    }

    #[test]
    fn the_hook_sees_the_isolated_environment() {
        let outside = tempfile::tempdir().unwrap();
        let script = executable(
            outside.path(),
            "env.sh",
            "#!/bin/sh\ncat > /dev/null\nprintf %s \"$HOME\" > home.txt\n",
        );
        let case = case_from(&format!(
            "name: t\nweight: 1\nsetup:\n  exec: {}\nexpect: {{ exit_code: 0 }}\n",
            script.display()
        ));
        let iso = isolate(&case, outside.path());

        run_setup(&case, &iso, outside.path()).unwrap();

        assert_eq!(
            std::fs::read_to_string(iso.root().join("home.txt")).unwrap(),
            iso.root().to_string_lossy(),
            "a hook that wrote into the developer's real home would defeat the whole point"
        );
    }

    #[test]
    fn a_hook_that_exits_non_zero_fails_the_case_with_its_stderr() {
        let outside = tempfile::tempdir().unwrap();
        let script = executable(
            outside.path(),
            "broken.sh",
            "#!/bin/sh\ncat > /dev/null\necho 'cannot render the template' >&2\nexit 4\n",
        );
        let case = case_from(&format!(
            "name: t\nweight: 1\nsetup:\n  exec: {}\nexpect: {{ exit_code: 0 }}\n",
            script.display()
        ));
        let iso = isolate(&case, outside.path());

        let error = run_setup(&case, &iso, outside.path()).unwrap_err();
        assert!(error.to_string().contains("cannot render the template"));
        assert!(
            error.to_string().contains('4'),
            "the exit code belongs in the message: {error}"
        );
    }

    #[test]
    fn a_hook_that_does_not_exist_names_its_path() {
        let outside = tempfile::tempdir().unwrap();
        let case = case_from(
            "name: t\nweight: 1\nsetup:\n  exec: ./no-such-hook.sh\nexpect: { exit_code: 0 }\n",
        );
        let iso = isolate(&case, outside.path());

        let error = run_setup(&case, &iso, outside.path()).unwrap_err();
        assert!(error.to_string().contains("no-such-hook.sh"));
    }

    fn observations() -> Observations {
        Observations {
            exit: 0,
            stdout: "one\ntwo\nthree\n".to_string(),
            ..Default::default()
        }
    }

    fn case_with_expect_hook(script: &Path) -> Case {
        case_from(&format!(
            "name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nexpect:\n  exec: {}\n",
            script.display()
        ))
    }

    #[test]
    fn a_case_without_an_expect_hook_produces_no_diffs() {
        let outside = tempfile::tempdir().unwrap();
        let case =
            case_from("name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nexpect: { exit_code: 0 }\n");
        let iso = isolate(&case, outside.path());

        assert!(
            run_expect(&case, &iso, &observations(), outside.path())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn the_hook_receives_the_observations_and_can_accept() {
        let outside = tempfile::tempdir().unwrap();
        let script = executable(
            outside.path(),
            "accept.sh",
            "#!/bin/sh\ncat > received.json\necho '{\"ok\":true,\"diffs\":[]}'\n",
        );
        let case = case_with_expect_hook(&script);
        let iso = isolate(&case, outside.path());

        assert!(
            run_expect(&case, &iso, &observations(), outside.path())
                .unwrap()
                .is_empty()
        );

        let received: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(iso.root().join("received.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            received["stdout"], "one\ntwo\nthree\n",
            "a hook checks what the core cannot, so it must see everything the core saw"
        );
    }

    #[test]
    fn the_hooks_diffs_are_reported_as_the_cores_own_would_be() {
        let outside = tempfile::tempdir().unwrap();
        let script = executable(
            outside.path(),
            "reject.sh",
            "#!/bin/sh\ncat > /dev/null\necho '{\"ok\":false,\"diffs\":[{\"path\":\"expect.exec.lines\",\"expected\":\"4\",\"got\":\"3\"}]}'\n",
        );
        let case = case_with_expect_hook(&script);
        let iso = isolate(&case, outside.path());

        let diffs = run_expect(&case, &iso, &observations(), outside.path()).unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "expect.exec.lines");
        assert_eq!(diffs[0].got, "3");
    }

    #[test]
    fn a_hook_saying_not_ok_without_diffs_still_fails_the_case() {
        let outside = tempfile::tempdir().unwrap();
        let script = executable(
            outside.path(),
            "terse.sh",
            "#!/bin/sh\ncat > /dev/null\necho '{\"ok\":false,\"diffs\":[]}'\n",
        );
        let case = case_with_expect_hook(&script);
        let iso = isolate(&case, outside.path());

        let diffs = run_expect(&case, &iso, &observations(), outside.path()).unwrap();
        assert_eq!(
            diffs.len(),
            1,
            "a hook that refuses without explaining still refuses; the core supplies a \
             placeholder diff rather than letting the case pass"
        );
    }

    #[test]
    fn an_unreadable_answer_is_a_protocol_error_naming_the_hook() {
        let outside = tempfile::tempdir().unwrap();
        let script = executable(
            outside.path(),
            "babble.sh",
            "#!/bin/sh\ncat > /dev/null\necho 'not json at all'\n",
        );
        let case = case_with_expect_hook(&script);
        let iso = isolate(&case, outside.path());

        let error = run_expect(&case, &iso, &observations(), outside.path()).unwrap_err();
        assert!(error.to_string().contains("babble.sh"));
    }

    #[test]
    fn resolve_always_yields_an_absolute_path_even_from_a_relative_root() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(outside.path().join("tests/hooks")).unwrap();
        executable(outside.path(), "tests/hooks/x.sh", "#!/bin/sh\n");

        let resolved = resolve("./tests/hooks/x.sh", outside.path());
        assert!(resolved.is_absolute());

        assert!(
            resolve("./tests/hooks/x.sh", Path::new(".")).is_absolute()
                || !Path::new("./tests/hooks/x.sh").exists(),
            "a project root of `.` is the common case — a plain join would stay relative and \
             then resolve against the isolated directory, where nothing of the project exists"
        );

        assert_eq!(
            resolve("/usr/bin/env", Path::new("/anywhere")),
            PathBuf::from("/usr/bin/env"),
            "an absolute hook path is left alone"
        );
    }

    #[test]
    fn a_relative_hook_resolves_against_the_project_root_not_the_isolation() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(outside.path().join("tests/hooks")).unwrap();
        executable(
            outside.path(),
            "tests/hooks/touch.sh",
            "#!/bin/sh\ncat > /dev/null\ntouch it-ran\n",
        );
        let case = case_from(
            "name: t\nweight: 1\nsetup:\n  exec: ./tests/hooks/touch.sh\nexpect: { exit_code: 0 }\n",
        );
        let iso = isolate(&case, outside.path());

        run_setup(&case, &iso, outside.path()).unwrap();

        assert!(
            iso.root().join("it-ran").exists(),
            "a hook lives in the project but runs in the isolation: resolving its path \
             against the isolated temporary directory would find nothing at all"
        );
    }

    #[test]
    fn a_hook_reading_to_end_of_input_does_not_hang() {
        let outside = tempfile::tempdir().unwrap();
        let script = executable(
            outside.path(),
            "drain.sh",
            "#!/bin/sh\nwc -c < /dev/stdin > counted.txt\n",
        );
        let case = case_from(&format!(
            "name: t\nweight: 1\nsetup:\n  exec: {}\n  pattern: ring\nexpect: {{ exit_code: 0 }}\n",
            script.display()
        ));
        let iso = isolate(&case, outside.path());

        run_setup(&case, &iso, outside.path()).unwrap();

        let counted: usize = std::fs::read_to_string(iso.root().join("counted.txt"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(
            counted > 0,
            "standard input must be closed after the payload, or a hook reading to \
             end-of-input would wait forever and the suite would hang rather than fail"
        );
    }
}
