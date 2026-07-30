//! The process adapter: run a command, read what it produced.
//!
//! This is the base case every other adapter is a specialisation of. On its own it covers
//! Rust, JavaScript/TypeScript, Python, Java and Kotlin: in all five the subject under
//! test is a process, and the fake binary is indifferent to the language of whoever calls
//! it. Only the shell needs an adapter of its own, because there a function is tested
//! rather than an executable.

use std::collections::BTreeMap;
use std::process::Command;

use gaveldrop_fake::Journal;

use crate::adapters::{Adapter, AdapterError};
use crate::{Case, Isolation, Observations};

/// Runs `setup.run` as a command line, with no shell involved.
///
/// The isolation's variables are substituted into every argument, which is what lets a case run the
/// project's **own** binary: the subject works in the isolated directory, so `./my-tool` looks for
/// something that is not there and `$GAVELDROP_PROJECT/my-tool` is what a case has to write. Testing
/// your own program is the most obvious thing anyone will try first.
///
/// A name isolation does not define is left literal, exactly as in `serve:` — a command is often a
/// shell script, and `${MYVAR-default}` is that shell's syntax rather than ours.
pub struct Process;

impl Adapter for Process {
    fn claims(&self, case: &Case) -> bool {
        case.setup
            .run
            .as_deref()
            .is_some_and(|argv| !argv.is_empty())
    }

    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError> {
        let argv = case
            .setup
            .run
            .as_deref()
            .filter(|argv| !argv.is_empty())
            .ok_or_else(|| AdapterError::Unsupported {
                case: case.name.clone(),
                reason: "setup has no `run` command line".to_string(),
            })?;

        let defined = iso.defined();
        let resolved: Vec<String> = argv
            .iter()
            .map(|argument| crate::iso::paths::expand_known(argument, &defined))
            .collect();

        let (program, arguments) =
            resolved
                .split_first()
                .ok_or_else(|| AdapterError::Unsupported {
                    case: case.name.clone(),
                    reason: "setup.run is empty".to_string(),
                })?;

        let mut command = Command::new(program);
        command.args(arguments).current_dir(iso.root());
        for (key, value) in iso.env() {
            command.env(key, value);
        }
        for key in iso.cleared() {
            command.env_remove(key);
        }

        let output = command.output().map_err(|source| AdapterError::Spawn {
            program: program.clone(),
            source,
        })?;

        Ok(Observations {
            exit: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            calls: Journal::read(&iso.journal_path())?,
            events: Vec::new(),
            files: iso.changes(),
            ext: BTreeMap::new(),
            ..Observations::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    fn case(yaml: &str) -> Case {
        Case::load_str(yaml, Path::new("inline")).unwrap()
    }

    fn fake_binary(dir: &Path) -> PathBuf {
        let path = dir.join("gaveldrop-fake");
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn isolate(case: &Case, outside: &Path, cleared: &[String]) -> Isolation {
        Isolation::prepare(case, &fake_binary(outside), &[], cleared, Path::new(".")).unwrap()
    }

    fn run(yaml: &str) -> Observations {
        let outside = tempfile::tempdir().unwrap();
        let case = case(yaml);
        let iso = isolate(&case, outside.path(), &[]);
        Process.invoke(&case, &iso).unwrap()
    }

    #[test]
    fn a_case_can_run_the_projects_own_binary() {
        let project = tempfile::tempdir().unwrap();
        let tool = project.path().join("my-tool");
        std::fs::write(&tool, "#!/bin/sh\nprintf 'my-tool 1.2.3'\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();

        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  run: [\"$GAVELDROP_PROJECT/my-tool\", \"--version\"]\nexpect: { exit_code: 0 }\n",
        );
        let iso = Isolation::prepare(
            &case,
            &fake_binary(outside.path()),
            &[],
            &[],
            project.path(),
        )
        .unwrap();

        let observed = Process.invoke(&case, &iso).unwrap();

        assert_eq!(
            observed.stdout, "my-tool 1.2.3",
            "testing your own program is the first thing anyone tries. The subject works in the \
             isolated directory, so `./my-tool` looks for something that is not there — without \
             substitution a case would need an absolute path, which cannot be committed"
        );
    }

    #[test]
    fn a_name_isolation_does_not_define_is_left_for_the_shell() {
        let observations = run(
            "name: t\nweight: 1\nsetup:\n  run: [\"sh\", \"-c\", \"printf %s \\\"${NOPE-fallback}\\\"\"]\nexpect: { exit_code: 0 }\n",
        );

        assert_eq!(
            observations.stdout.trim(),
            "fallback",
            "`${{NOPE-fallback}}` is the shell's syntax for a default, not ours. Substituting it \
             would reject a legitimate command for using a construct we never owned"
        );
    }

    #[test]
    fn the_exit_code_and_both_streams_are_observed() {
        let observations = run(
            "name: t\nweight: 1\nsetup:\n  run: [\"sh\", \"-c\", \"echo out; echo err >&2; exit 3\"]\nexpect: { exit_code: 3 }\n",
        );

        assert_eq!(observations.exit, 3);
        assert_eq!(observations.stdout.trim(), "out");
        assert_eq!(observations.stderr.trim(), "err");
    }

    #[test]
    fn the_subject_runs_inside_the_isolated_root() {
        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  run: [\"sh\", \"-c\", \"pwd\"]\nexpect: { exit_code: 0 }\n",
        );
        let iso = isolate(&case, outside.path(), &[]);
        let observations = Process.invoke(&case, &iso).unwrap();

        let reported = std::fs::canonicalize(observations.stdout.trim()).unwrap();
        let expected = std::fs::canonicalize(iso.root()).unwrap();
        assert_eq!(
            reported, expected,
            "the working directory must be the isolated root, so a relative path in a \
             case cannot reach the developer's checkout"
        );
    }

    #[test]
    fn the_home_variable_the_subject_sees_is_the_isolated_one() {
        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  run: [\"sh\", \"-c\", \"printf %s \\\"$HOME\\\"\"]\nexpect: { exit_code: 0 }\n",
        );
        let iso = isolate(&case, outside.path(), &[]);
        let observations = Process.invoke(&case, &iso).unwrap();

        assert_eq!(
            observations.stdout.trim(),
            iso.root().to_string_lossy(),
            "this is the load-bearing invariant: a case must never see the real home"
        );
    }

    #[test]
    fn cleared_variables_are_absent_from_the_subject_environment() {
        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  run: [\"sh\", \"-c\", \"printf %s \\\"${MYTOOL_CONFIG_DIR-absent}\\\"\"]\nexpect: { exit_code: 0 }\n",
        );
        let iso = isolate(&case, outside.path(), &["MYTOOL_CONFIG_DIR".to_string()]);
        let observations = Process.invoke(&case, &iso).unwrap();

        assert_eq!(
            observations.stdout.trim(),
            "absent",
            "a variable listed for clearing must not reach the subject, whatever the \
             developer's own environment holds"
        );
    }

    #[test]
    fn calls_are_read_back_from_the_journal() {
        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  run: [\"sh\", \"-c\", \"true\"]\nexpect: { exit_code: 0 }\n",
        );
        let iso = isolate(&case, outside.path(), &[]);

        gaveldrop_fake::Journal::new(iso.journal_path())
            .record(&gaveldrop_fake::Call {
                bin: "git".into(),
                args: vec!["status".into()],
                call: 1,
                key: "git".into(),
                catch_all: false,
                passthrough: false,
                exit: 0,
            })
            .unwrap();

        let observations = Process.invoke(&case, &iso).unwrap();
        assert_eq!(observations.calls.len(), 1);
        assert_eq!(observations.calls[0].bin, "git");
    }

    #[test]
    fn a_missing_journal_observes_no_calls_rather_than_failing() {
        let observations = run(
            "name: t\nweight: 1\nsetup:\n  run: [\"sh\", \"-c\", \"true\"]\nexpect: { exit_code: 0 }\n",
        );
        assert!(
            observations.calls.is_empty(),
            "a subject that called nobody is a legitimate observation"
        );
    }

    #[test]
    fn a_program_that_cannot_start_is_an_error_not_a_panic() {
        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  run: [\"no-such-program-anywhere\"]\nexpect: { exit_code: 0 }\n",
        );
        let iso = isolate(&case, outside.path(), &[]);

        let error = Process.invoke(&case, &iso).unwrap_err();
        assert!(
            error.to_string().contains("no-such-program-anywhere"),
            "a case that cannot start must become a failed case with a diagnostic, never \
             a panic that takes the other ninety-nine with it: {error}"
        );
    }

    #[test]
    fn a_case_with_no_run_is_rejected_by_this_adapter() {
        let outside = tempfile::tempdir().unwrap();
        let case =
            case("name: t\nweight: 1\nsetup:\n  exec: ./prepare.sh\nexpect: { exit_code: 0 }\n");
        let iso = isolate(&case, outside.path(), &[]);

        let error = Process.invoke(&case, &iso).unwrap_err();
        assert!(
            error.to_string().contains("run"),
            "the process adapter needs a command line; `exec:` alone is the setup hook's \
             job and that hook is not built yet: {error}"
        );
    }

    #[test]
    fn the_extension_map_stays_empty_for_a_plain_process() {
        let observations = run(
            "name: t\nweight: 1\nsetup:\n  run: [\"sh\", \"-c\", \"true\"]\nexpect: { exit_code: 0 }\n",
        );
        assert!(
            observations.ext.is_empty(),
            "`ext` is for what a technology alone can produce. A process produces nothing \
             that has no named field already, so this adapter must leave it empty rather \
             than treat it as a junk drawer"
        );
    }
}
