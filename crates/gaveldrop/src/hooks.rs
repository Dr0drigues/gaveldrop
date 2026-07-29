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

use std::process::{Command, Output, Stdio};

use crate::{Case, Isolation};

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
pub fn run_setup(case: &Case, iso: &Isolation) -> Result<(), HookError> {
    let Some(path) = case.setup.exec.as_deref() else {
        return Ok(());
    };

    let payload = setup_payload(case).map_err(|source| HookError::Protocol {
        which: "setup",
        path: path.to_string(),
        source,
    })?;

    let output = feed(path, "setup", iso, &payload)?;
    refuse_on_failure("setup", path, &output)?;

    Ok(())
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

/// Spawns `path` inside the isolation, writes `payload` to its standard input, and waits.
///
/// Standard input is closed after the write: without that, a hook reading to end-of-input
/// would wait forever, and the suite would hang rather than fail.
fn feed(
    path: &str,
    which: &'static str,
    iso: &Isolation,
    payload: &[u8],
) -> Result<Output, HookError> {
    let mut command = Command::new(path);
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
    use std::path::{Path, PathBuf};

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
        Isolation::prepare(case, &fake_binary(outside), &[], &[]).unwrap()
    }

    #[test]
    fn a_case_without_a_setup_hook_does_nothing_at_all() {
        let outside = tempfile::tempdir().unwrap();
        let case =
            case_from("name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nexpect: { exit_code: 0 }\n");
        let iso = isolate(&case, outside.path());

        assert!(run_setup(&case, &iso).is_ok());
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

        run_setup(&case, &iso).unwrap();

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

        run_setup(&case, &iso).unwrap();

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

        let error = run_setup(&case, &iso).unwrap_err();
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

        let error = run_setup(&case, &iso).unwrap_err();
        assert!(error.to_string().contains("no-such-hook.sh"));
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

        run_setup(&case, &iso).unwrap();

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
