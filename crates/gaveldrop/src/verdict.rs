//! Evaluating expectations against observations, and the verdict that comes out.

pub mod calls;
pub mod events;
pub mod files;
pub mod headers;
pub mod invariants;
pub mod json;
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
    /// The invariants the project declared, by name.
    pub invariants: invariants::NamedInvariants,
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
    let mut diffs = check(&case.expect, observations, context, "expect");
    diffs.extend(check_steps(case, observations, context));

    let no_files = BTreeMap::new();
    let expected_files = case.expect.files.as_ref().unwrap_or(&no_files);
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

/// Checks one `Expect` against one set of observations, rooting every path at `at`.
///
/// Extracted so a step is checked by exactly the same code as the run as a whole. Two evaluators
/// would drift, and then an expectation would quietly mean one thing at the top level and another
/// inside a step — the one property this project cannot afford to lose.
fn check(
    expect: &crate::Expect,
    observations: &Observations,
    context: &Context,
    at: &str,
) -> Vec<Diff> {
    let mut diffs = Vec::new();

    if let Some(want) = expect.exit_code
        && want != observations.exit
    {
        diffs.push(Diff {
            path: format!("{at}.exit_code"),
            expected: want.to_string(),
            got: observations.exit.to_string(),
        });
    }

    if let Some(expectation) = &expect.stdout {
        diffs.extend(text::check(
            expectation,
            &observations.stdout,
            &format!("{at}.stdout"),
        ));
    }
    if let Some(expectation) = &expect.stderr {
        diffs.extend(text::check(
            expectation,
            &observations.stderr,
            &format!("{at}.stderr"),
        ));
    }
    if let Some(expected) = &expect.calls {
        diffs.extend(calls::check(expected, &observations.calls));
    }

    diffs.extend(events::check_subsequence(
        &expect.events,
        &observations.events,
    ));
    if let Some(expected) = &expect.event_counts {
        diffs.extend(events::check_counts(expected, &observations.events));
    }

    for name in &expect.invariants {
        match context.invariants.get(name) {
            Some(shape) => diffs.extend(invariants::check(shape, name, &observations.events)),
            None => diffs.push(Diff {
                path: format!("{at}.invariants.{name}"),
                expected: "an invariant the project declared".to_string(),
                got: format!(
                    "{name} appears in no `invariants:` block. Declare it in gaveldrop.yaml, \
                     or fix the spelling"
                ),
            }),
        }
    }

    if let Some(want) = expect.status
        && Some(want) != observations.status
    {
        diffs.push(Diff {
            path: format!("{at}.status"),
            expected: want.to_string(),
            got: match observations.status {
                Some(seen) => seen.to_string(),
                None => "no response at all".to_string(),
            },
        });
    }

    if let Some(expected) = &expect.headers {
        diffs.extend(headers::check(expected, &observations.headers, at));
    }

    if let Some(expectation) = &expect.body {
        diffs.extend(text::check(
            expectation,
            &observations.body,
            &format!("{at}.body"),
        ));
    }

    let no_files = BTreeMap::new();
    diffs.extend(files::check(
        expect.files.as_ref().unwrap_or(&no_files),
        &observations.files,
        &context.defined,
    ));

    diffs
}

/// Checks every declared step against the exchange that answered it.
///
/// A count mismatch is a failure in both directions. Too few observed means the subject stopped
/// halfway and comparing only what came back would show green — the worst outcome available. Too many
/// means an exchange happened that the case never declared, which is the same class of surprise as an
/// unexpected call.
fn check_steps(case: &Case, observations: &Observations, context: &Context) -> Vec<Diff> {
    let mut diffs = Vec::new();

    for (index, step) in case.steps.iter().enumerate() {
        let at = match &step.name {
            Some(name) => format!("steps[{index}] \"{name}\""),
            None => format!("steps[{index}]"),
        };

        match observations.steps.get(index) {
            Some(seen) => diffs.extend(check(&step.expect, seen, context, &at)),
            None => diffs.push(Diff {
                path: format!("steps[{index}]"),
                expected: "the exchange happens".to_string(),
                got: format!(
                    "the case declares {} exchanges and {} were performed",
                    case.steps.len(),
                    observations.steps.len()
                ),
            }),
        }
    }

    if observations.steps.len() > case.steps.len() {
        diffs.push(Diff {
            path: "steps".to_string(),
            expected: format!("{} exchanges", case.steps.len()),
            got: format!("{} were performed", observations.steps.len()),
        });
    }

    diffs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(expect_yaml: &str) -> Case {
        let yaml = format!("name: t\nweight: 5\nsetup: {{ run: [\"true\"] }}\n{expect_yaml}");
        Case::load_str(&yaml, std::path::Path::new("inline")).unwrap()
    }

    fn evaluate_with(
        case: &Case,
        observations: &Observations,
        declared: &invariants::NamedInvariants,
    ) -> Outcome {
        evaluate_in(
            case,
            observations,
            &Context {
                defined: BTreeMap::new(),
                invariants: declared.clone(),
            },
        )
    }

    fn stepped(steps_yaml: &str) -> Case {
        let yaml =
            format!("name: t\nweight: 5\nsetup: {{ run: [\"true\"] }}\nexpect: {{}}\n{steps_yaml}");
        Case::load_str(&yaml, std::path::Path::new("inline")).unwrap()
    }

    fn saw(stdout: &str) -> Observations {
        Observations {
            stdout: stdout.to_string(),
            ..Observations::default()
        }
    }

    fn answered(status: u16, body: &str, headers: &[(&str, &str)]) -> Observations {
        Observations {
            status: Some(status),
            body: body.to_string(),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                .collect(),
            ..Observations::default()
        }
    }

    #[test]
    fn a_status_mismatch_reports_both_numbers() {
        let case = case("expect: { status: 201 }");
        let outcome = evaluate(&case, &answered(500, "", &[]));

        let diff = &outcome.diffs[0];
        assert_eq!(diff.path, "expect.status");
        assert_eq!(diff.expected, "201");
        assert_eq!(
            diff.got, "500",
            "both numbers, because `expected 201` alone leaves the reader running the case by \
             hand to find out what came back"
        );
    }

    #[test]
    fn a_header_comparison_ignores_the_case_of_the_name() {
        let case = case("expect:\n  headers:\n    Content-Type: { contains: [\"json\"] }");
        let outcome = evaluate(
            &case,
            &answered(200, "", &[("content-type", "application/json")]),
        );

        assert!(
            outcome.passed,
            "HTTP header names are case-insensitive. A case asserting `Content-Type` against a \
             server sending `content-type` would be testing the server's spelling rather than its \
             behaviour: {:?}",
            outcome.diffs
        );
    }

    #[test]
    fn a_header_the_response_never_sent_is_a_failure_naming_it() {
        let case = case("expect:\n  headers:\n    X-Request-Id: { contains: [\"-\"] }");
        let outcome = evaluate(&case, &answered(200, "", &[]));

        assert!(
            outcome.diffs[0].path.contains("X-Request-Id"),
            "an absent header must name itself, or the reader cannot tell a missing header from a \
             wrong value: {:?}",
            outcome.diffs
        );
    }

    #[test]
    fn a_body_expectation_reuses_the_text_expectation_shape() {
        let case =
            case("expect:\n  body:\n    contains: [\"\\\"id\\\"\"]\n    absent: [\"error\"]");
        let outcome = evaluate(&case, &answered(200, "{\"error\":\"nope\"}", &[]));

        let paths: Vec<&str> = outcome
            .diffs
            .iter()
            .map(|diff| diff.path.as_str())
            .collect();
        assert!(
            paths.contains(&"expect.body.contains[0]") && paths.contains(&"expect.body.absent[0]"),
            "the same `contains`/`absent` as stdout, so a reader who knows one knows both and the \
             indexed paths come for free: {paths:?}"
        );
    }

    #[test]
    fn a_non_web_case_asserts_nothing_about_a_response() {
        let case = case("expect: { exit_code: 0 }");
        let outcome = evaluate(&case, &Observations::default());

        assert!(
            outcome.passed,
            "three expectations no process case mentions must cost it nothing. A subject that \
             produced no status must not start failing because a status field exists: {:?}",
            outcome.diffs
        );
    }

    #[test]
    fn a_status_expected_where_no_response_came_back_is_a_failure() {
        let case = case("expect: { status: 200 }");
        let outcome = evaluate(&case, &Observations::default());

        assert!(
            !outcome.passed,
            "a case asking about a status when nothing answered must fail rather than pass by \
             default: silence is not a 200"
        );
    }

    #[test]
    fn a_step_can_assert_on_its_own_response() {
        let case = Case::load_str(
            "name: t\nweight: 5\nsetup: { run: [\"true\"] }\nexpect: {}\nsteps:\n  - name: creates\n    expect: { status: 201 }\n",
            std::path::Path::new("inline"),
        )
        .unwrap();
        let observations = Observations {
            steps: vec![answered(500, "", &[])],
            ..Observations::default()
        };

        let outcome = evaluate(&case, &observations);
        assert!(
            outcome.diffs[0]
                .path
                .starts_with("steps[0] \"creates\".status"),
            "the response expectations work per step through the same evaluator, so nothing here \
             is web-specific plumbing: {:?}",
            outcome.diffs[0].path
        );
    }

    #[test]
    fn a_case_with_steps_still_has_its_own_expect_checked() {
        let case = Case::load_str(
            "name: t\nweight: 5\nsetup: { run: [\"true\"] }\nexpect: { exit_code: 0 }\nsteps:\n  \
             - expect: { stdout: { contains: [\"one\"] } }\n",
            std::path::Path::new("inline"),
        )
        .unwrap();
        let observations = Observations {
            exit: 3,
            steps: vec![saw("one")],
            ..Observations::default()
        };

        let outcome = evaluate(&case, &observations);
        assert!(
            outcome
                .diffs
                .iter()
                .any(|diff| diff.path == "expect.exit_code"),
            "`expect` describes what the run as a whole produced and `steps[].expect` what one \
             exchange did. Adding steps must not silence the first: {:?}",
            outcome.diffs
        );
    }

    #[test]
    fn a_failing_step_names_its_index_and_its_name() {
        let case = stepped(
            "steps:\n  - name: creates the order\n    expect: { stdout: { contains: [\"created\"] } }\n",
        );
        let observations = Observations {
            steps: vec![saw("nothing of the sort")],
            ..Observations::default()
        };

        let outcome = evaluate(&case, &observations);
        let path = outcome
            .diffs
            .first()
            .map(|diff| diff.path.clone())
            .unwrap_or_default();

        assert!(
            path.contains("steps[0]") && path.contains("creates the order"),
            "a reader must know *which* exchange broke without counting lines in the YAML, so the \
             path carries both the index and the name the case gave it. Got {path:?}"
        );
    }

    #[test]
    fn a_step_without_a_name_still_names_its_index() {
        let case = stepped("steps:\n  - expect: { stdout: { contains: [\"absent\"] } }\n");
        let observations = Observations {
            steps: vec![saw("")],
            ..Observations::default()
        };

        let outcome = evaluate(&case, &observations);
        assert!(
            outcome.diffs[0].path.starts_with("steps[0]"),
            "naming a step is optional; locating it is not: {:?}",
            outcome.diffs[0].path
        );
    }

    #[test]
    fn fewer_observed_steps_than_declared_is_a_failure_not_a_silent_pass() {
        let case = stepped(
            "steps:\n  - expect: { stdout: { contains: [\"first\"] } }\n  - expect: { stdout: { contains: [\"second\"] } }\n",
        );
        let observations = Observations {
            steps: vec![saw("first")],
            ..Observations::default()
        };

        let outcome = evaluate(&case, &observations);
        assert!(
            !outcome.passed,
            "an adapter that ran one exchange out of two must fail the case. Comparing only what \
             came back would make a case that silently stopped halfway look green, which is the \
             worst outcome available"
        );
        assert!(
            outcome.diffs.iter().any(|diff| diff.path == "steps[1]"),
            "and it must name the step that never ran: {:?}",
            outcome.diffs
        );
    }

    #[test]
    fn more_observed_steps_than_declared_is_also_a_failure() {
        let case = stepped("steps:\n  - expect: {}\n");
        let observations = Observations {
            steps: vec![saw("one"), saw("two")],
            ..Observations::default()
        };

        assert!(
            !evaluate(&case, &observations).passed,
            "an exchange the case never declared happened anyway, which is the same class of \
             surprise as an unexpected call: silence about it would hide a subject doing more than \
             the case says"
        );
    }

    fn result_event() -> events::Event {
        events::Event {
            kind: "result".to_string(),
            fields: BTreeMap::new(),
        }
    }

    fn single_result() -> invariants::NamedInvariants {
        [(
            "single_result".to_string(),
            invariants::InvariantShape::ExactlyOne {
                kind: "result".to_string(),
            },
        )]
        .into_iter()
        .collect()
    }

    #[test]
    fn a_case_uses_an_invariant_by_the_name_its_project_gave_it() {
        let case = case("expect:\n  exit_code: 0\n  invariants: [single_result]\n");
        let mut observations = observed(0, "", "");
        observations.events = vec![result_event()];

        assert!(evaluate_with(&case, &observations, &single_result()).passed);

        observations.events.clear();
        let outcome = evaluate_with(&case, &observations, &single_result());
        assert!(!outcome.passed);
        assert_eq!(outcome.diffs[0].path, "expect.invariants.single_result");
    }

    #[test]
    fn an_invariant_the_project_never_declared_is_a_case_failure_not_a_silent_pass() {
        let case = case("expect:\n  exit_code: 0\n  invariants: [never_declared]\n");
        let outcome = evaluate_with(&case, &observed(0, "", ""), &Default::default());

        assert!(!outcome.passed);
        assert!(
            outcome.diffs[0].got.contains("never_declared"),
            "naming an invariant that does not exist must fail loudly: silently skipping it \
             would turn a typo into a case that checks nothing. Got: {:?}",
            outcome.diffs[0]
        );
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
