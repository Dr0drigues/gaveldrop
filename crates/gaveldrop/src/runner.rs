//! The driver: for each case, isolate, invoke, evaluate, report.

use std::path::Path;

use crate::adapters::Adapter;
use crate::config::ConfigError;
use crate::report::Sink;
use crate::verdict::events::{self, Event};
use crate::verdict::{Context, evaluate_in};
use crate::{Case, Config, Diff, Isolation, Outcome, Process, Report};

/// Runs every case the configuration finds, feeding `sink` as each one finishes.
///
/// Never panics on a broken case. A temporary directory that refuses to be created, a
/// program that will not start, a case that will not parse — each becomes a failed outcome
/// with a diagnostic, not a panic that takes the other ninety-nine with it.
///
/// The distinction that decides which is which: a **load** error is loud and stops
/// everything, because a suite that cannot find its cases must not report success. An
/// **execution** error is a case failure like any other.
pub fn run_all(
    config: &Config,
    root: &Path,
    fake_binary: &Path,
    sink: &mut dyn Sink,
) -> Result<Report, ConfigError> {
    let paths = config.discover(root)?;
    let mut outcomes = Vec::with_capacity(paths.len());

    for path in paths {
        let outcome = match Case::load(&path) {
            Ok(case) => run_one(&case, fake_binary, config),
            Err(error) => setup_failure(&path.to_string_lossy(), 0, error.to_string()),
        };
        sink.case_finished(&outcome);
        outcomes.push(outcome);
    }

    let report = Report::from(outcomes);
    sink.finish(&report);
    Ok(report)
}

/// Isolates, invokes and evaluates one case.
fn run_one(case: &Case, fake_binary: &Path, config: &Config) -> Outcome {
    let mut iso = match Isolation::prepare(case, fake_binary, &config.fake.bins, &config.clear_env)
    {
        Ok(iso) => iso,
        Err(error) => return setup_failure(&case.name, case.weight, error.to_string()),
    };

    iso.snapshot();

    let context = Context {
        defined: iso.defined(),
    };

    match Process.invoke(case, &iso) {
        Ok(mut observations) => {
            observations.events = read_events(&observations.stdout, config);
            evaluate_in(case, &observations, &context)
        }
        Err(error) => setup_failure(&case.name, case.weight, error.to_string()),
    }
}

/// The structured events the project's configuration says to look for.
///
/// Done here rather than in the adapter: an adapter invokes and observes, and it has no
/// business knowing how this project spells its event types. A project that declares no
/// `events:` block gets none, which costs it nothing.
fn read_events(stdout: &str, config: &Config) -> Vec<Event> {
    config
        .events
        .as_ref()
        .map(|events| events::extract(stdout, events))
        .unwrap_or_default()
}

/// An outcome for a failure that happened before any expectation could be checked.
///
/// Reported the same way as a mismatch so callers never need to special-case it — the
/// difference is in the diff's path, which says `setup` rather than `expect.…`.
///
/// A case that would not parse has no trustworthy `name:`, so callers pass its path
/// instead: a report that says "the case called nothing failed" helps nobody find the file.
fn setup_failure(name: &str, weight: u32, reason: String) -> Outcome {
    Outcome {
        name: name.to_string(),
        weight,
        allow_fail: false,
        passed: false,
        diffs: vec![Diff {
            path: "setup".to_string(),
            expected: "the case runs".to_string(),
            got: reason,
        }],
        unexpected_calls: Vec::new(),
        unmentioned_files: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Collects what a run reports, in the order it reports it.
    struct Recorder {
        cases: Vec<String>,
        finished: bool,
    }

    impl Sink for Recorder {
        fn case_finished(&mut self, outcome: &Outcome) {
            assert!(
                !self.finished,
                "finish() must come after every case, or a live renderer would draw a \
                 summary and then keep going"
            );
            self.cases.push(outcome.name.clone());
        }

        fn finish(&mut self, _report: &Report) {
            self.finished = true;
        }
    }

    fn project(cases: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("tests/cases")).unwrap();
        for (name, body) in cases {
            std::fs::write(dir.path().join("tests/cases").join(name), body).unwrap();
        }

        let fake = dir.path().join("gaveldrop-fake");
        std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        dir
    }

    fn config() -> Config {
        Config {
            cases: "tests/cases/**/*.yaml".to_string(),
            ..Default::default()
        }
    }

    fn drive(dir: &tempfile::TempDir) -> (Report, Recorder) {
        let mut recorder = Recorder {
            cases: Vec::new(),
            finished: false,
        };
        let report = run_all(
            &config(),
            dir.path(),
            &dir.path().join("gaveldrop-fake"),
            &mut recorder,
        )
        .unwrap();
        (report, recorder)
    }

    const PASSING: &str = "name: passing\nweight: 5\nsetup:\n  run: [\"sh\", \"-c\", \"true\"]\nexpect: { exit_code: 0 }\n";
    const FAILING: &str = "name: failing\nweight: 3\nsetup:\n  run: [\"sh\", \"-c\", \"exit 7\"]\nexpect: { exit_code: 0 }\n";

    #[test]
    fn every_discovered_case_is_run_and_reported_in_order() {
        let dir = project(&[("a-passing.yaml", PASSING), ("b-failing.yaml", FAILING)]);
        let (report, recorder) = drive(&dir);

        assert_eq!(recorder.cases, vec!["passing", "failing"]);
        assert!(recorder.finished, "finish() must be called once at the end");
        assert_eq!(report.summary().total, 2);
        assert_eq!(report.summary().score, 5);
        assert!(!report.is_success());
    }

    #[test]
    fn a_case_that_will_not_parse_becomes_a_failed_case_not_an_aborted_run() {
        let dir = project(&[
            ("a-broken.yaml", "this is not a case at all\n"),
            ("b-passing.yaml", PASSING),
        ]);
        let (report, recorder) = drive(&dir);

        assert_eq!(
            recorder.cases.len(),
            2,
            "the broken case must be reported and the run must continue: a load failure \
             on one file cannot take the other ninety-nine with it"
        );
        assert!(!report.outcomes[0].passed);
        assert_eq!(
            report.outcomes[0].diffs[0].path, "setup",
            "a failure before any expectation could be checked is located at `setup`, \
             which is what distinguishes it from a mismatch"
        );
        assert!(report.outcomes[1].passed);
    }

    #[test]
    fn a_broken_case_is_named_by_its_path_since_it_has_no_usable_name() {
        let dir = project(&[("unreadable.yaml", "name: [not a string]\n")]);
        let (report, _) = drive(&dir);

        assert!(
            report.outcomes[0].name.contains("unreadable.yaml"),
            "a case that would not parse has no trustworthy `name:`, so the report falls \
             back to the path. Got: {}",
            report.outcomes[0].name
        );
    }

    #[test]
    fn a_subject_that_cannot_start_becomes_a_failed_case_with_the_program_named() {
        let dir = project(&[(
            "missing-program.yaml",
            "name: no-such\nweight: 4\nsetup:\n  run: [\"no-such-program-anywhere\"]\nexpect: { exit_code: 0 }\n",
        )]);
        let (report, _) = drive(&dir);

        assert!(!report.outcomes[0].passed);
        assert_eq!(report.outcomes[0].weight, 4, "the weight must survive");
        assert!(
            report.outcomes[0].diffs[0]
                .got
                .contains("no-such-program-anywhere")
        );
    }

    #[test]
    fn a_tolerated_failure_keeps_the_run_successful() {
        let dir = project(&[(
            "known.yaml",
            "name: known\nweight: 3\nallow_fail: true\nsetup:\n  run: [\"sh\", \"-c\", \"exit 1\"]\nexpect: { exit_code: 0 }\n",
        )]);
        let (report, _) = drive(&dir);

        assert!(!report.outcomes[0].passed);
        assert!(
            report.is_success(),
            "a declared, tolerated failure must not fail the run"
        );
    }

    #[test]
    fn a_pattern_matching_nothing_stops_the_run_rather_than_reporting_success() {
        let dir = project(&[]);
        let mut recorder = Recorder {
            cases: Vec::new(),
            finished: false,
        };

        let error = run_all(
            &config(),
            dir.path(),
            &dir.path().join("gaveldrop-fake"),
            &mut recorder,
        )
        .unwrap_err();

        assert!(error.to_string().contains("tests/cases"));
        assert!(
            !recorder.finished,
            "a load error is loud and stops everything; it must not produce an empty \
             green report"
        );
    }
}
