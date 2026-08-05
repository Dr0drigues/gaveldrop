//! The shell adapter: source files, call a function, observe what it did.

pub mod line;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use crate::adapters::{Adapter, AdapterError};
use crate::{Case, Isolation, Journal, Observations, Snapshot};

/// Runs a shell function with its files sourced first.
///
/// The only technology of the six where the subject is a function rather than an executable. What
/// makes that cheap is that the core never learns a word of it: `shell`, `source` and `call` arrive
/// through `Setup::extra`, which is opaque by design.
pub struct Shell;

impl Adapter for Shell {
    fn name(&self) -> &str {
        "shell"
    }

    fn claims(&self, case: &Case) -> bool {
        case.setup.extra.contains_key("shell")
    }

    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError> {
        if case.steps.is_empty() {
            let mut once = one_call(case, iso, &strings(case, "call"), iso.limit())?;
            once.files = iso.changes();
            return Ok(once);
        }

        Ok(gathered(each_step(case, iso)?, iso))
    }
}

/// What the run as a whole produced, from the exchanges that produced it.
///
/// A stepped case invokes **once per step and no more**. An extra whole-case invocation would run the
/// subject three times for two steps, and the first step would then be judged on a tree the
/// invocation before it had already changed — which is how the idempotence assertion silently stops
/// meaning anything.
///
/// So the whole-case fields are gathered: the outputs and the calls concatenated in order, the exit
/// code of the last exchange, and the files from the runner's own snapshot, which is everything the
/// case wrote rather than what any single step did.
///
/// The calls used to be the last exchange's, which was right only because the journal was cumulative.
/// Each exchange now keeps its own, so the run adds them up. See `Ledger`.
fn gathered(steps: Vec<Observations>, iso: &Isolation) -> Observations {
    let exit = steps.last().map(|last| last.exit).unwrap_or(0);
    let stdout = steps.iter().map(|seen| seen.stdout.as_str()).collect();
    let stderr = steps.iter().map(|seen| seen.stderr.as_str()).collect();
    let calls = steps.iter().flat_map(|seen| seen.calls.clone()).collect();

    // The case's own limit when any exchange ran out of it. This was absent entirely: a stepped shell
    // case whose function hung was killed and then reported no `timeout` diff at all, because the
    // field lived on the exchange and nothing carried it up. The reader got an exit code of -1 and no
    // reason for it.
    let timed_out_after_ms = steps
        .iter()
        .any(|seen| seen.timed_out_after_ms.is_some())
        .then(|| iso.limit().map(|limit| limit.as_millis() as u64))
        .flatten();

    Observations {
        exit,
        stdout,
        stderr,
        calls,
        files: iso.changes(),
        steps,
        timed_out_after_ms,
        ..Observations::default()
    }
}

/// One invocation per declared step, each seeing only what it wrote itself.
///
/// This is how a function is shown to be idempotent: two steps with the same `call:`, the second
/// declaring `no_new_files: true`. Each step gets its own snapshot, so the second is judged on what
/// *it* changed rather than on what the first left behind — without which the assertion would be
/// meaningless.
///
/// A step may name its own `call:`; otherwise it repeats the one in `setup`, which is what makes the
/// idempotence case read as two identical invocations rather than as a duplicated block.
fn each_step(case: &Case, iso: &Isolation) -> Result<Vec<Observations>, AdapterError> {
    let fallback = strings(case, "call");
    let mut performed = Vec::with_capacity(case.steps.len());
    let mut ledger = crate::adapters::Ledger::new();
    let budget = iso.budget();

    for step in &case.steps {
        // One budget for the case, shared by its exchanges. See `Budget`.
        if budget.spent() {
            break;
        }

        let call = step
            .request
            .get("call")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|argv| !argv.is_empty())
            .unwrap_or_else(|| fallback.clone());

        let before = Snapshot::take(iso.root());
        let mut seen = one_call(case, iso, &call, budget.left())?;
        seen.files = before.changes_since(iso.root());
        ledger.only_the_new(&mut seen.calls);

        // This adapter honours no `capture:`: a shell function answers text, and deciding that
        // its output is a JSON document to walk by path would be inventing a meaning for the
        // format rather than implementing one. Reported as missed rather than ignored, so a case
        // that declares one is told at `capture.<name>` instead of failing later on a name that
        // silently stayed literal.
        for (name, path) in &step.capture {
            seen.missed_captures.insert(name.clone(), path.clone());
        }

        performed.push(seen);
    }

    Ok(performed)
}

/// Sources the case's files and invokes `call` once, within `limit`.
///
/// `limit` is passed in rather than read from the isolation: a case's exchanges share one budget, so
/// what this invocation may take is whatever the ones before it left. See `Budget`.
fn one_call(
    case: &Case,
    iso: &Isolation,
    call: &[String],
    limit: Option<std::time::Duration>,
) -> Result<Observations, AdapterError> {
    let shell = string(case, "shell")?;
    let sources = resolved(&strings(case, "source"), iso.project_root());

    let mut command = Command::new(&shell);
    command
        .arg("-c")
        .arg(line::assemble(&sources, call))
        .current_dir(iso.root());
    for (key, value) in iso.env() {
        command.env(key, value);
    }
    for key in iso.cleared() {
        command.env_remove(key);
    }

    let completed = crate::adapters::invoke(&mut command, case.setup.stdin.as_deref(), limit)
        .map_err(|source| AdapterError::Spawn {
            program: shell,
            source,
        })?;

    Ok(Observations {
        exit: completed.output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&completed.output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&completed.output.stderr).into_owned(),
        calls: Journal::read(&iso.journal_path())?,
        events: Vec::new(),
        files: Vec::new(),
        ext: BTreeMap::new(),
        timed_out_after_ms: completed.timed_out_after_ms,
        ..Observations::default()
    })
}

/// Turns each declared source into a path that still means the same file once the subject's working
/// directory is the isolated root.
///
/// A case says `source: ["functions/ui.zsh"]` and means a file of the project — that file *is* the
/// subject. Left relative it would be looked up inside the isolation, where it does not exist, and
/// the shell would report exit 127 with nothing on standard output. So a relative path resolves
/// against the project root; an absolute one is left alone, because someone naming an absolute path
/// meant it.
fn resolved(sources: &[String], project_root: &Path) -> Vec<String> {
    sources
        .iter()
        .map(|declared| {
            let path = Path::new(declared);
            if path.is_absolute() {
                return declared.clone();
            }
            let joined = project_root.join(path);
            std::fs::canonicalize(&joined)
                .unwrap_or(joined)
                .to_string_lossy()
                .into_owned()
        })
        .collect()
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
    use std::path::PathBuf;

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
        Isolation::prepare(case, &fake_binary(outside), bins, &[], outside).unwrap()
    }

    fn stepped(yaml: &str) -> Case {
        Case::load_str(yaml, Path::new("inline")).unwrap()
    }

    #[test]
    fn a_second_identical_call_is_judged_on_what_it_wrote_itself() {
        let outside = tempfile::tempdir().unwrap();
        let case = stepped(
            "name: t\nweight: 1\nsetup:\n  shell: bash\n  source: [\"once.sh\"]\n  call: [\"once\"]\nexpect: {}\nsteps:\n  - name: first call creates it\n    expect: {}\n  - name: second call adds nothing\n    expect: { no_new_files: true }\n",
        );
        let iso = isolate(&case, outside.path(), &[]);
        std::fs::write(
            iso.project_root().join("once.sh"),
            "once() { [ -f marker ] || printf x > marker; }\n",
        )
        .unwrap();

        let observed = Shell.invoke(&case, &iso).unwrap();

        assert_eq!(observed.steps.len(), 2);
        assert!(
            !observed.steps[0].files.is_empty(),
            "the first call created the marker and must be seen doing it"
        );
        assert!(
            observed.steps[1].files.is_empty(),
            "the second call changed nothing, and it must be judged on that rather than on what \
             the first left behind. Without a snapshot per step the assertion would be \
             meaningless: {:?}",
            observed.steps[1].files
        );
    }

    /// A shell exchange keeps its own calls too, which is a second call site of the same rule.
    ///
    /// Tested here rather than trusted to the process adapter's test: the two `each_step` loops are
    /// separate code, and a rule applied to one of two paths is the shape of defect this project keeps
    /// finding. The function appends to the journal itself, as the fake binary would.
    #[test]
    fn a_shell_exchange_counts_its_own_calls_and_not_the_earlier_ones() {
        let outside = tempfile::tempdir().unwrap();
        let case = stepped(
            "name: t\nweight: 1\nsetup:\n  shell: bash\n  source: [\"calls.sh\"]\n  call: [\"tool\"]\nexpect: {}\nsteps:\n  - expect: {}\n  - expect: {}\n",
        );
        let iso = isolate(&case, outside.path(), &[]);
        std::fs::write(
            iso.project_root().join("calls.sh"),
            "tool() { printf '%s\\n' '{\"bin\":\"outil\",\"args\":[],\"call\":1,\"key\":\"outil\",\"catch_all\":false,\"passthrough\":false,\"exit\":0}' >> journal.jsonl; }\n",
        )
        .unwrap();

        let observed = Shell.invoke(&case, &iso).unwrap();

        assert_eq!(observed.steps[0].calls.len(), 1);
        assert_eq!(
            observed.steps[1].calls.len(),
            1,
            "the second exchange called once and used to be told 2: {:?}",
            observed.steps[1].calls
        );
        assert_eq!(observed.calls.len(), 2, "and the run called twice");
    }

    #[test]
    fn a_function_that_appends_every_time_is_caught_by_the_second_step() {
        let outside = tempfile::tempdir().unwrap();
        let case = stepped(
            "name: t\nweight: 1\nsetup:\n  shell: bash\n  source: [\"twice.sh\"]\n  call: [\"twice\"]\nexpect: {}\nsteps:\n  - expect: {}\n  - expect: { no_new_files: true }\n",
        );
        let iso = isolate(&case, outside.path(), &[]);
        std::fs::write(
            iso.project_root().join("twice.sh"),
            "twice() { printf 'more\\n' >> appended; }\n",
        )
        .unwrap();

        let observed = Shell.invoke(&case, &iso).unwrap();

        assert!(
            !observed.steps[1].files.is_empty(),
            "a function appending on every call is exactly the bug `no_new_files` exists to catch — \
             the classic one being a configuration function that adds to PATH twice"
        );
    }

    #[test]
    fn a_step_may_name_its_own_call_instead_of_repeating_the_setups() {
        let outside = tempfile::tempdir().unwrap();
        let case = stepped(
            "name: t\nweight: 1\nsetup:\n  shell: bash\n  source: [\"two.sh\"]\n  call: [\"first\"]\nexpect: {}\nsteps:\n  - expect: {}\n  - request: { call: [\"second\"] }\n    expect: {}\n",
        );
        let iso = isolate(&case, outside.path(), &[]);
        std::fs::write(
            iso.project_root().join("two.sh"),
            "first() { echo one; }\nsecond() { echo two; }\n",
        )
        .unwrap();

        let observed = Shell.invoke(&case, &iso).unwrap();

        assert_eq!(observed.steps[0].stdout.trim(), "one");
        assert_eq!(
            observed.steps[1].stdout.trim(),
            "two",
            "a step repeats setup's call by default, which is what makes the idempotence case read \
             as two identical invocations — but it may name another"
        );
    }

    #[test]
    fn a_case_without_steps_still_reports_its_files_as_before() {
        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  shell: bash\n  source: [\"w.sh\"]\n  call: [\"w\"]\nexpect: { exit_code: 0 }\n",
        );
        let iso = isolate(&case, outside.path(), &[]);
        std::fs::write(
            iso.project_root().join("w.sh"),
            "w() { printf x > written; }\n",
        )
        .unwrap();

        let observed = Shell.invoke(&case, &iso).unwrap();

        assert!(
            observed.steps.is_empty() && !observed.files.is_empty(),
            "adding steps must not change what a case without them observes: {:?}",
            observed.files
        );
    }

    #[test]
    fn a_relative_source_resolves_against_the_project_and_not_the_isolation() {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("here.zsh"), "f() { :; }\n").unwrap();

        let resolved = resolved(&["here.zsh".to_string()], project.path());

        assert_eq!(
            std::fs::canonicalize(&resolved[0]).unwrap(),
            std::fs::canonicalize(project.path().join("here.zsh")).unwrap(),
            "a case naming `functions/ui.zsh` means a file of the project, because that file is \
             the subject. Left relative it would be looked up inside the isolation, where it does \
             not exist, and the shell would report exit 127 with nothing on standard output"
        );
    }

    #[test]
    fn an_absolute_source_is_left_alone() {
        let resolved = resolved(&["/etc/hosts".to_string()], Path::new("/nowhere"));

        assert_eq!(
            resolved[0], "/etc/hosts",
            "someone naming an absolute path meant it, and joining it under the project root \
             would produce a path that exists nowhere"
        );
    }

    #[test]
    fn a_sourced_function_is_called_with_its_arguments() {
        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  shell: bash\n  source: [\"greet.sh\"]\n  call: [\"greet\", \"world\"]\nexpect: { exit_code: 0 }\n",
        );
        let iso = isolate(&case, outside.path(), &[]);
        std::fs::write(
            iso.project_root().join("greet.sh"),
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
        std::fs::write(
            iso.project_root().join("h.sh"),
            "h() { printf %s \"$HOME\"; }\n",
        )
        .unwrap();

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
        std::fs::write(iso.project_root().join("first.sh"), "prefix=one\n").unwrap();
        std::fs::write(
            iso.project_root().join("second.sh"),
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
    fn a_capture_this_adapter_cannot_honour_is_reported_rather_than_ignored() {
        let outside = tempfile::tempdir().unwrap();
        let case = case(
            "name: t\nweight: 1\nsetup:\n  shell: bash\n  call: [\"true\"]\nsteps:\n  - name: one\n    request: { call: [\"true\"] }\n    capture: { order_id: data.order.id }\n    expect: {}\nexpect: {}\n",
        );
        let iso = isolate(&case, outside.path(), &[]);

        let observed = Shell.invoke(&case, &iso).unwrap();

        assert_eq!(
            observed.steps[0]
                .missed_captures
                .get("order_id")
                .map(String::as_str),
            Some("data.order.id"),
            "the format offers `capture:` on any step and this adapter honours none, so a case \
             declaring one used to get silence: nothing captured, nothing said, and `$order_id` \
             literal in the next request. Reported, it becomes a failure at `capture.order_id`"
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
            iso.project_root().join("k.sh"),
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
