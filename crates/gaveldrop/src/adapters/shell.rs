//! The shell adapter: source files, call a function, observe what it did.

pub mod line;

use std::collections::BTreeMap;
use std::process::Command;

use crate::adapters::{Adapter, AdapterError};
use crate::{Case, Isolation, Journal, Observations};

/// Runs a shell function with its files sourced first.
///
/// The only technology of the six where the subject is a function rather than an executable. What
/// makes that cheap is that the core never learns a word of it: `shell`, `source` and `call` arrive
/// through `Setup::extra`, which is opaque by design.
pub struct Shell;

impl Adapter for Shell {
    fn claims(&self, case: &Case) -> bool {
        case.setup.extra.contains_key("shell")
    }

    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError> {
        let shell = string(case, "shell")?;
        let sources = strings(case, "source");
        let call = strings(case, "call");

        let mut command = Command::new(&shell);
        command
            .arg("-c")
            .arg(line::assemble(&sources, &call))
            .current_dir(iso.root());
        for (key, value) in iso.env() {
            command.env(key, value);
        }
        for key in iso.cleared() {
            command.env_remove(key);
        }

        let output = command.output().map_err(|source| AdapterError::Spawn {
            program: shell,
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
        })
    }
}

/// A string value from the opaque part of `setup`.
fn string(case: &Case, key: &str) -> Result<String, AdapterError> {
    case.setup
        .extra
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| AdapterError::Unsupported {
            case: case.name.clone(),
            reason: format!("setup has no `{key}` naming the shell to run"),
        })
}

/// A list of strings from the opaque part of `setup`, empty when absent.
fn strings(case: &Case, key: &str) -> Vec<String> {
    case.setup
        .extra
        .get(key)
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
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

    fn isolate(case: &Case, outside: &Path, bins: &[String]) -> Isolation {
        Isolation::prepare(case, &fake_binary(outside), bins, &[]).unwrap()
    }

    #[test]
    fn a_sourced_function_is_called_with_its_arguments() {
        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  shell: bash\n  source: [\"greet.sh\"]\n  call: [\"greet\", \"world\"]\nexpect: { exit_code: 0 }\n",
        );
        let iso = isolate(&case, outside.path(), &[]);
        std::fs::write(
            iso.root().join("greet.sh"),
            "greet() { printf 'hello %s' \"$1\"; }\n",
        )
        .unwrap();

        let observed = Shell.invoke(&case, &iso).unwrap();
        assert_eq!(observed.stdout, "hello world");
        assert_eq!(observed.exit, 0);
    }

    #[test]
    fn the_function_runs_under_the_isolated_home() {
        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  shell: bash\n  source: [\"h.sh\"]\n  call: [\"h\"]\nexpect: { exit_code: 0 }\n",
        );
        let iso = isolate(&case, outside.path(), &[]);
        std::fs::write(iso.root().join("h.sh"), "h() { printf %s \"$HOME\"; }\n").unwrap();

        let observed = Shell.invoke(&case, &iso).unwrap();
        assert_eq!(
            observed.stdout.trim(),
            iso.root().to_string_lossy(),
            "the load-bearing invariant does not weaken because the subject is a function"
        );
    }

    #[test]
    fn the_sources_are_loaded_in_the_order_the_case_gave_them() {
        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  shell: bash\n  source: [\"first.sh\", \"second.sh\"]\n  call: [\"show\"]\nexpect: { exit_code: 0 }\n",
        );
        let iso = isolate(&case, outside.path(), &[]);
        std::fs::write(iso.root().join("first.sh"), "prefix=one\n").unwrap();
        std::fs::write(
            iso.root().join("second.sh"),
            "show() { printf '%s-two' \"$prefix\"; }\n",
        )
        .unwrap();

        let observed = Shell.invoke(&case, &iso).unwrap();
        assert_eq!(
            observed.stdout, "one-two",
            "the order is load-bearing for a real shell project: a function file that uses the \
             UI library must be sourced after it"
        );
    }

    #[test]
    fn a_case_without_a_shell_is_not_this_adapters_business() {
        let outside = tempfile::tempdir().unwrap();
        let case =
            case("name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nexpect: { exit_code: 0 }\n");
        assert!(!Shell.claims(&case));

        let error = Shell
            .invoke(&case, &isolate(&case, outside.path(), &[]))
            .unwrap_err();
        assert!(
            error.to_string().contains("shell"),
            "and invoking it anyway must say which key is missing: {error}"
        );
    }

    #[test]
    fn a_source_that_does_not_exist_fails_the_case_rather_than_the_run() {
        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  shell: bash\n  source: [\"absent.sh\"]\n  call: [\"nothing\"]\nexpect: { exit_code: 0 }\n",
        );
        let iso = isolate(&case, outside.path(), &[]);

        let observed = Shell.invoke(&case, &iso).unwrap();
        assert_ne!(observed.exit, 0);
        assert!(
            observed.stderr.contains("absent.sh"),
            "a missing file must name itself on standard error, or the reader has no idea what \
             was not found: {}",
            observed.stderr
        );
    }

    #[test]
    fn a_shell_that_is_not_installed_is_an_error_not_a_panic() {
        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  shell: no-such-shell-anywhere\n  call: [\"x\"]\nexpect: { exit_code: 0 }\n",
        );
        let error = Shell
            .invoke(&case, &isolate(&case, outside.path(), &[]))
            .unwrap_err();
        assert!(
            error.to_string().contains("no-such-shell-anywhere"),
            "a machine without the shell a case asks for must produce a diagnostic naming it, \
             never a panic: {error}"
        );
    }

    #[test]
    fn a_function_finds_the_faked_binary_and_not_the_real_one() {
        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  shell: bash\n  source: [\"k.sh\"]\n  call: [\"k\"]\nexpect: { exit_code: 0 }\n",
        );
        let iso = isolate(&case, outside.path(), &["kubectl".to_string()]);
        std::fs::write(
            iso.root().join("k.sh"),
            "k() { kubectl get pods; printf ':%s' \"$?\"; not-faked-at-all; printf ':%s' \"$?\"; }\n",
        )
        .unwrap();

        let observed = Shell.invoke(&case, &iso).unwrap();
        assert_eq!(
            observed.stdout, ":0:127",
            "a function's dependencies are faked exactly as a binary's are — that is why the fake \
             is a binary on PATH rather than a library. The faked call must succeed and an \
             unfaked one must still be not-found, or the search path was inherited rather than \
             built: {}",
            observed.stdout
        );
    }
}
