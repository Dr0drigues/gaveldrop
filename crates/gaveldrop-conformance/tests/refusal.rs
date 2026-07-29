//! The kit run against adapters that are deliberately wrong.
//!
//! Running the kit against a conformant adapter shows it can say **yes**. That is not the property
//! that matters: a kit whose checks all silently held would look exactly the same. These two
//! adapters each break one thing, and each must be caught by the check that guards it — and only by
//! that one. A kit that failed everything would be no more useful than a kit that passed
//! everything.
//!
//! They also live outside the crate on purpose. Compiling them proves a third party can write an
//! adapter against the published API without reaching for anything private.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "panicking is how a test reports failure; an integration test file is its own crate"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use assert_cmd::cargo::cargo_bin;
use gaveldrop::adapters::{Adapter, AdapterError};
use gaveldrop::{Case, Isolation, Observations, Process};
use gaveldrop_conformance::ConformanceReport;

/// An adapter that runs the subject in the isolated directory but with the ambient environment.
///
/// The plausible mistake: it looks correct, reports the exit code and both streams faithfully, and
/// sees the files. Only the environment is the developer's own — so the subject reads the real home
/// and the real search path.
struct Leaky;

impl Adapter for Leaky {
    fn claims(&self, case: &Case) -> bool {
        case.setup.run.is_some()
    }

    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError> {
        let output = spawn(case, iso, |_command| {})?;
        Ok(observations(output, iso))
    }
}

/// An adapter that builds the isolated environment correctly and forgets to apply `clear_env`.
///
/// A subtler mistake than [`Leaky`], and a likelier one: everything about isolation is right except
/// the one list a case uses to say "this variable must not exist here".
struct Forgetful;

impl Adapter for Forgetful {
    fn claims(&self, case: &Case) -> bool {
        case.setup.run.is_some()
    }

    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError> {
        let output = spawn(case, iso, |command| {
            for (key, value) in iso.env() {
                command.env(key, value);
            }
        })?;
        Ok(observations(output, iso))
    }
}

fn spawn(
    case: &Case,
    iso: &Isolation,
    environment: impl FnOnce(&mut Command),
) -> Result<std::process::Output, AdapterError> {
    let argv = case.setup.run.as_deref().unwrap_or_default();
    let (program, arguments) = argv
        .split_first()
        .ok_or_else(|| AdapterError::Unsupported {
            case: case.name.clone(),
            reason: "setup has no `run` command line".to_string(),
        })?;

    let mut command = Command::new(program);
    command.args(arguments).current_dir(iso.root());
    environment(&mut command);

    command.output().map_err(|source| AdapterError::Spawn {
        program: program.clone(),
        source,
    })
}

fn observations(output: std::process::Output, iso: &Isolation) -> Observations {
    Observations {
        exit: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        calls: gaveldrop::Journal::read(&iso.journal_path()).unwrap_or_default(),
        events: Vec::new(),
        files: iso.changes(),
        ext: BTreeMap::new(),
    }
}

fn fake_binary() -> PathBuf {
    let path = cargo_bin("gaveldrop-fake");
    assert!(
        path.is_file(),
        "{} is missing. Run `cargo test --workspace` rather than testing this crate alone.",
        path.display()
    );
    path
}

fn failed(report: &ConformanceReport) -> Vec<&str> {
    report
        .findings
        .iter()
        .filter(|finding| !finding.held)
        .map(|finding| finding.check.name)
        .collect()
}

#[test]
fn an_adapter_that_leaks_the_environment_is_refused() {
    let report = gaveldrop_conformance::run(&Leaky, &fake_binary());

    assert!(
        !report.is_conformant(),
        "an adapter that lets the subject read the developer's real home must be refused, or the \
         kit certifies the one defect it exists to prevent:\n{}",
        report.render()
    );
}

#[test]
fn the_environment_leak_is_caught_by_the_checks_that_guard_it() {
    let report = gaveldrop_conformance::run(&Leaky, &fake_binary());
    let caught = failed(&report);

    for guarding in [
        "the_home_directory_is_the_isolated_one",
        "an_unexpected_call_reaches_the_catch_all",
    ] {
        assert!(
            caught.contains(&guarding),
            "`{guarding}` must catch an adapter that hands the subject the ambient environment, \
             or it is not watching what it claims:\n{}",
            report.render()
        );
    }

    for unrelated in [
        "exit_code_is_reported",
        "both_streams_are_reported",
        "files_written_are_reported",
    ] {
        assert!(
            !caught.contains(&unrelated),
            "`{unrelated}` has nothing to do with the environment and must still hold. A report \
             naming everything is as useless for repairing an adapter as one naming nothing:\n{}",
            report.render()
        );
    }
}

/// Why `a_cleared_variable_does_not_reach_the_subject` is absent from both lists above.
///
/// An adapter that leaks the ambient environment leaks whatever that environment happens to hold,
/// and `XDG_CONFIG_HOME` is set on GitHub's Linux runners and unset on macOS. So the check fails
/// here or not depending on the machine — correctly in both cases, which is precisely why the test
/// above cannot assert on it either way. Pinning it would encode the developer's shell into the
/// suite. [`Forgetful`] carries the exactness proof instead: it sets the isolated environment
/// itself, so it depends on nothing outside.
#[test]
fn an_adapter_that_ignores_clear_env_is_refused_and_only_there() {
    let report = gaveldrop_conformance::run(&Forgetful, &fake_binary());

    assert_eq!(
        failed(&report),
        vec!["a_cleared_variable_does_not_reach_the_subject"],
        "this adapter gets isolation right and only skips `clear_env`, so exactly one check may \
         fail. If none does, that check is vacant: a variable no environment defines is removed \
         whether the adapter tries or not, so the check must clear one the isolation itself \
         sets:\n{}",
        report.render()
    );
}

#[test]
fn the_reference_adapter_still_passes_what_the_broken_ones_fail() {
    let report = gaveldrop_conformance::run(&Process, &fake_binary());

    assert!(
        report.is_conformant(),
        "the checks must discriminate, not merely be strict: a kit that refuses every adapter \
         including the correct one measures nothing:\n{}",
        report.render()
    );
}

#[test]
fn a_refusal_prints_what_the_check_protects_and_what_was_seen() {
    let rendered = gaveldrop_conformance::run(&Leaky, &fake_binary()).render();

    assert!(
        rendered.contains("load-bearing invariant"),
        "a refusal must carry the reason the check exists, or whoever wrote the adapter has to \
         read our source to learn what broke:\n{rendered}"
    );
    assert!(
        rendered.starts_with("FAIL"),
        "and the failures must come first: a reader scanning a refusal should not have to walk \
         past the checks that held:\n{rendered}"
    );
}
