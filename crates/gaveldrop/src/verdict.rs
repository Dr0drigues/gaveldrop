//! Evaluating expectations against observations, and the verdict that comes out.

pub mod calls;
pub mod events;
pub mod files;
pub mod text;

use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use crate::{Case, Observations};

/// One failed assertion.
///
/// `path` is where it came from in the case document. The core needs no line numbers, but
/// pull-request annotation and editor squiggles will, and going from a path to a line is
/// easy whereas reconstructing a provenance you did not keep is not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diff {
    /// Where the assertion sits in the case, such as `expect.stdout.contains[1]`.
    pub path: String,
    /// What the case asked for.
    pub expected: String,
    /// What the run produced.
    pub got: String,
}

/// The verdict on one case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    /// The case's name.
    pub name: String,
    /// The case's weight.
    pub weight: u32,
    /// Whether this failure is tolerated.
    pub allow_fail: bool,
    /// Whether every assertion held.
    pub passed: bool,
    /// The assertions that did not.
    pub diffs: Vec<Diff>,
    /// Binaries that reached the catch-all.
    pub unexpected_calls: Vec<String>,
    /// Files the subject wrote that the case says nothing about.
    ///
    /// Offered as help, never counted as a failure: it is often where you discover what you
    /// should have been asserting.
    #[serde(default)]
    pub unmentioned_files: Vec<String>,
}

/// Everything the evaluation needs beyond the case and the observations.
///
/// Carried as one struct rather than a growing parameter list: later batches add invariants
/// here, and a field addition breaks nothing whereas a signature change breaks every caller.
#[derive(Debug, Default)]
pub struct Context {
    /// The variables a case may use in a path.
    pub defined: std::collections::BTreeMap<String, String>,
}

/// Evaluates `case` against `observations`, with no project context.
///
/// Convenient where a case uses neither paths nor, later, invariants. Not a shortcut worth
/// taking anywhere a project's configuration is available.
pub fn evaluate(case: &Case, observations: &Observations) -> Outcome {
    evaluate_in(case, observations, &Context::default())
}

/// Evaluates `case` against `observations`, with the project's context.
///
/// An omitted expectation is not checked. A case says what it cares about, and silence is
/// not a claim — which is what keeps a case readable instead of exhaustive.
pub fn evaluate_in(case: &Case, observations: &Observations, context: &Context) -> Outcome {
    let mut diffs = Vec::new();

    if let Some(want) = case.expect.exit_code
        && want != observations.exit
    {
        diffs.push(Diff {
            path: "expect.exit_code".to_string(),
            expected: want.to_string(),
            got: observations.exit.to_string(),
        });
    }

    if let Some(expectation) = &case.expect.stdout {
        diffs.extend(text::check(
            expectation,
            &observations.stdout,
            "expect.stdout",
        ));
    }
    if let Some(expectation) = &case.expect.stderr {
        diffs.extend(text::check(
            expectation,
            &observations.stderr,
            "expect.stderr",
        ));
    }
    if let Some(expected) = &case.expect.calls {
        diffs.extend(calls::check(expected, &observations.calls));
    }

    let no_files = BTreeMap::new();
    let expected_files = case.expect.files.as_ref().unwrap_or(&no_files);
    diffs.extend(files::check(
        expected_files,
        &observations.files,
        &context.defined,
    ));

    let unexpected_calls = calls::unexpected(&observations.calls);

    Outcome {
        name: case.name.clone(),
        weight: case.weight,
        allow_fail: case.allow_fail,
        passed: diffs.is_empty() && unexpected_calls.is_empty(),
        diffs,
        unexpected_calls,
        unmentioned_files: files::unmentioned(
            expected_files,
            &observations.files,
            &context.defined,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(expect_yaml: &str) -> Case {
        let yaml = format!("name: t\nweight: 5\nsetup: {{ run: [\"true\"] }}\n{expect_yaml}");
        Case::load_str(&yaml, std::path::Path::new("inline")).unwrap()
    }

    fn observed(exit: i32, stdout: &str, stderr: &str) -> Observations {
        Observations {
            exit,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            ..Default::default()
        }
    }

    fn call(bin: &str, catch_all: bool) -> gaveldrop_fake::Call {
        gaveldrop_fake::Call {
            bin: bin.to_string(),
            args: Vec::new(),
            call: 1,
            key: bin.to_string(),
            catch_all,
            passthrough: false,
            exit: 0,
        }
    }

    #[test]
    fn a_matching_run_passes_and_carries_the_case_metadata() {
        let outcome = evaluate(&case("expect: { exit_code: 0 }\n"), &observed(0, "", ""));

        assert!(outcome.passed);
        assert!(outcome.diffs.is_empty());
        assert_eq!(outcome.name, "t");
        assert_eq!(outcome.weight, 5);
        assert!(!outcome.allow_fail);
    }

    #[test]
    fn a_wrong_exit_code_yields_a_diff_that_names_its_own_key() {
        let outcome = evaluate(&case("expect: { exit_code: 0 }\n"), &observed(3, "", ""));

        assert!(!outcome.passed);
        assert_eq!(outcome.diffs.len(), 1);
        assert_eq!(
            outcome.diffs[0].path, "expect.exit_code",
            "every assertion carries the path it came from: pull-request annotation and \
             editor squiggles both need it later, and a provenance you did not keep \
             cannot be reconstructed"
        );
        assert_eq!(outcome.diffs[0].expected, "0");
        assert_eq!(outcome.diffs[0].got, "3");
    }

    #[test]
    fn contains_and_absent_are_indexed_in_their_paths() {
        let outcome = evaluate(
            &case(
                "expect:\n  stdout:\n    contains: [\"present\", \"missing\"]\n    absent: [\"forbidden\"]\n",
            ),
            &observed(0, "present and forbidden", ""),
        );

        let paths: Vec<&str> = outcome.diffs.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["expect.stdout.contains[1]", "expect.stdout.absent[0]"]
        );
    }

    #[test]
    fn stderr_expectations_are_checked_too() {
        let outcome = evaluate(
            &case("expect:\n  stderr:\n    contains: [\"dirty repository\"]\n"),
            &observed(1, "", "error: dirty repository"),
        );
        assert!(outcome.passed, "diffs: {:?}", outcome.diffs);
    }

    #[test]
    fn an_absent_expectation_reports_what_it_found_not_just_that_it_failed() {
        let outcome = evaluate(
            &case("expect:\n  stdout:\n    absent: [\"ZSH_ENV\"]\n"),
            &observed(0, "scriptPath: $ZSH_ENV_DIR/scripts/fmt.zsh", ""),
        );

        assert!(!outcome.passed);
        assert!(
            outcome.diffs[0].got.contains("ZSH_ENV_DIR"),
            "a failure must show the offending value, or the report sends you back to the \
             code to find out what happened"
        );
    }

    #[test]
    fn call_counts_are_checked_against_the_journal() {
        let case = case("expect:\n  exit_code: 0\n  calls:\n    git: 2\n    gh: 0\n");
        let mut observations = observed(0, "", "");
        observations.calls = vec![call("git", false), call("git", false)];

        assert!(evaluate(&case, &observations).passed);
    }

    #[test]
    fn a_call_count_that_is_off_names_the_binary_in_its_path() {
        let case = case("expect:\n  exit_code: 0\n  calls:\n    git: 2\n");
        let mut observations = observed(0, "", "");
        observations.calls = vec![call("git", false)];

        let outcome = evaluate(&case, &observations);
        assert!(!outcome.passed);
        assert_eq!(outcome.diffs[0].path, "expect.calls.git");
        assert_eq!(outcome.diffs[0].expected, "2");
        assert_eq!(outcome.diffs[0].got, "1");
    }

    #[test]
    fn a_declared_count_of_zero_catches_a_dependency_that_should_not_have_been_touched() {
        let case = case("expect:\n  exit_code: 0\n  calls:\n    payments: 0\n");
        let mut observations = observed(0, "", "");
        observations.calls = vec![call("payments", false)];

        assert!(
            !evaluate(&case, &observations).passed,
            "asserting a dependency was never called is often the interesting half: the \
             answer can be right while the side effect was wrong"
        );
    }

    #[test]
    fn a_catch_all_call_fails_the_case_even_with_no_calls_expectation() {
        let case = case("expect: { exit_code: 0 }\n");
        let mut observations = observed(0, "", "");
        observations.calls = vec![call("kubectl", true)];

        let outcome = evaluate(&case, &observations);
        assert!(
            !outcome.passed,
            "an unexpected call must fail the case whether or not the case mentions \
             calls: that is what the catch-all exists for"
        );
        assert_eq!(outcome.unexpected_calls, vec!["kubectl".to_string()]);
    }

    #[test]
    fn an_omitted_expectation_is_not_checked() {
        let outcome = evaluate(&case("expect: {}\n"), &observed(42, "anything", "anything"));
        assert!(
            outcome.passed,
            "an absent key asserts nothing. A case says what it cares about; silence is \
             not a claim"
        );
    }

    #[test]
    fn a_long_offending_line_is_truncated_so_one_line_cannot_drown_a_report() {
        let noise = "x".repeat(500);
        let outcome = evaluate(
            &case("expect:\n  stdout:\n    contains: [\"needle\"]\n"),
            &observed(0, &noise, ""),
        );

        assert!(
            outcome.diffs[0].got.chars().count() < 200,
            "got was {} characters long",
            outcome.diffs[0].got.chars().count()
        );
    }
}
