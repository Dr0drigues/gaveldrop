//! Aggregating outcomes, and rendering them.

pub mod annotate;
pub mod html;
pub mod jsonl;
pub mod junit;
pub mod lines;
pub mod merge;
pub mod terminal;

use serde::{Deserialize, Serialize};

use crate::{Diff, Outcome};

/// Every outcome of a run.
///
/// **A list of outcomes plus a summary computed from it — never a frozen summary.** That
/// is what makes two reports mergeable by plain concatenation, and therefore what will let
/// a suite spread across several machines without touching the format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Report {
    /// The outcomes, in the order the cases finished.
    pub outcomes: Vec<Outcome>,
}

/// The counts and the weighted score, derived from the outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// How many cases ran.
    pub total: usize,
    /// How many passed.
    pub passed: usize,
    /// How many failed without being tolerated.
    pub failed: usize,
    /// How many failed but were declared `allow_fail`.
    pub tolerated: usize,
    /// Sum of the weights of passing cases.
    pub score: u32,
    /// Sum of every weight.
    pub max_score: u32,
}

impl From<Vec<Outcome>> for Report {
    fn from(outcomes: Vec<Outcome>) -> Self {
        Self { outcomes }
    }
}

impl Report {
    /// Computes the counts and the weighted score.
    pub fn summary(&self) -> Summary {
        let mut summary = Summary {
            total: self.outcomes.len(),
            passed: 0,
            failed: 0,
            tolerated: 0,
            score: 0,
            max_score: 0,
        };

        for outcome in &self.outcomes {
            summary.max_score = summary.max_score.saturating_add(outcome.weight);
            if outcome.passed {
                summary.passed += 1;
                summary.score = summary.score.saturating_add(outcome.weight);
            } else if outcome.allow_fail {
                summary.tolerated += 1;
            } else {
                summary.failed += 1;
            }
        }

        summary
    }

    /// Whether the run should be considered a success.
    pub fn is_success(&self) -> bool {
        self.summary().failed == 0
    }

    /// Concatenates reports. Merging is concatenation precisely because no summary is
    /// stored.
    pub fn merge<I: IntoIterator<Item = Self>>(reports: I) -> Self {
        Self {
            outcomes: reports
                .into_iter()
                .flat_map(|report| report.outcomes)
                .collect(),
        }
    }
}

/// Somewhere outcomes are rendered as they arrive.
///
/// Outcomes are emitted **one per finished case**, not only aggregated at the end. A report
/// that exists only once the suite has finished forecloses any live rendering — an editor
/// ticking off its cases, a terminal showing a failure the moment it lands.
pub trait Sink {
    /// Called as soon as one case has a verdict.
    fn case_finished(&mut self, outcome: &Outcome);
    /// Called once, after the last case.
    fn finish(&mut self, report: &Report);
}

/// Feeds several renderers from one run.
///
/// A case must reach the terminal **and** the machine-readable file while it is still running.
/// Putting the loop here rather than in the facade keeps the facade free of logic, which is the
/// invariant that lets a Rust project test everything the binary does.
///
/// Carries a lifetime so a renderer may borrow its output — a test writing into a `Vec<u8>` is not
/// `'static`, and demanding that it be would make the tee untestable without a file.
#[derive(Default)]
pub struct Tee<'a> {
    sinks: Vec<Box<dyn Sink + 'a>>,
}

impl<'a> Tee<'a> {
    /// A tee with no renderers. Harmless until one is added.
    pub fn new() -> Self {
        Self { sinks: Vec::new() }
    }

    /// Adds a renderer.
    pub fn add(&mut self, sink: Box<dyn Sink + 'a>) {
        self.sinks.push(sink);
    }
}

impl Sink for Tee<'_> {
    fn case_finished(&mut self, outcome: &Outcome) {
        for sink in &mut self.sinks {
            sink.case_finished(outcome);
        }
    }

    fn finish(&mut self, report: &Report) {
        for sink in &mut self.sinks {
            sink.finish(report);
        }
    }
}

/// The failure lines for one outcome, ready to print.
///
/// Extracted so every renderer words a failure the same way: the expectation path, what
/// was wanted, what was found.
pub fn failure_lines(outcome: &Outcome) -> Vec<String> {
    let mut lines: Vec<String> = outcome
        .diffs
        .iter()
        .map(
            |Diff {
                 path,
                 expected,
                 got,
             }| {
                format!("    {path}\n      expected  {expected}\n      got       {got}")
            },
        )
        .collect();

    if !outcome.unexpected_calls.is_empty() {
        lines.push(format!(
            "    unexpected calls\n      got       {}",
            outcome.unexpected_calls.join(", ")
        ));
    }

    if !outcome.unmentioned_files.is_empty() {
        lines.push(format!(
            "    (also written, not asserted: {})",
            outcome.unmentioned_files.join(", ")
        ));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(name: &str, weight: u32, passed: bool, allow_fail: bool) -> Outcome {
        Outcome {
            name: name.to_string(),
            weight,
            allow_fail,
            passed,
            diffs: Vec::new(),
            unexpected_calls: Vec::new(),
            unmentioned_files: Vec::new(),
        }
    }

    fn rendered_for(outcomes: Vec<Outcome>, call_finish: bool) -> String {
        let report = Report::from(outcomes);
        let mut buffer = Vec::new();
        {
            let mut sink = terminal::Terminal::plain(&mut buffer);
            for outcome in &report.outcomes {
                sink.case_finished(outcome);
            }
            if call_finish {
                sink.finish(&report);
            }
        }
        String::from_utf8_lossy(&buffer).into_owned()
    }

    #[test]
    fn the_score_weighs_passing_cases_against_the_total() {
        let report = Report::from(vec![
            outcome("a", 8, true, false),
            outcome("b", 5, true, false),
            outcome("c", 3, false, false),
        ]);
        let summary = report.summary();

        assert_eq!(summary.total, 3);
        assert_eq!(summary.passed, 2);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.score, 13);
        assert_eq!(summary.max_score, 16);
    }

    #[test]
    fn a_tolerated_failure_is_counted_apart_rather_than_hidden() {
        let report = Report::from(vec![
            outcome("a", 5, true, false),
            outcome("known", 3, false, true),
        ]);
        let summary = report.summary();

        assert_eq!(summary.tolerated, 1);
        assert_eq!(
            summary.failed, 0,
            "a tolerated case must not count as a failure, or the suite could never be \
             green"
        );
        assert!(
            !report.outcomes[1].passed,
            "and it must still be visibly not passing: tolerated is not hidden"
        );
        assert!(report.is_success());
    }

    #[test]
    fn two_reports_merge_by_concatenation() {
        let first = Report::from(vec![outcome("a", 5, true, false)]);
        let second = Report::from(vec![outcome("b", 3, false, false)]);

        let merged = Report::merge([first, second]);
        assert_eq!(
            merged.summary().total,
            2,
            "the report is a list of outcomes plus a summary computed from it, never a \
             frozen summary: that is what will let a suite spread across machines and be \
             consolidated afterwards"
        );
        assert_eq!(merged.summary().score, 5);
    }

    #[test]
    fn an_empty_report_has_a_zero_score_and_does_not_divide_by_anything() {
        let summary = Report::from(Vec::new()).summary();
        assert_eq!(summary.total, 0);
        assert_eq!(summary.score, 0);
        assert_eq!(summary.max_score, 0);
    }

    #[test]
    fn the_terminal_sink_emits_a_case_before_finish_is_ever_called() {
        let midway = rendered_for(vec![outcome("first", 5, true, false)], false);

        assert!(
            midway.contains("first"),
            "outcomes must be emitted as they happen, not only aggregated at the end: a \
             report that exists only once the suite has finished forecloses any live \
             rendering. Got: {midway}"
        );
        assert!(
            !midway.contains("cases ·"),
            "and this must hold with finish() never called at all, or the test proves \
             nothing about when the line was written. Got: {midway}"
        );
    }

    #[test]
    fn the_terminal_report_names_the_case_the_expectation_and_the_value() {
        let failing = Outcome {
            diffs: vec![Diff {
                path: "expect.stdout.absent[0]".to_string(),
                expected: "nowhere: \"ZSH_ENV\"".to_string(),
                got: "scriptPath: $ZSH_ENV_DIR/scripts/fmt.zsh".to_string(),
            }],
            ..outcome("k9s-leaves-no-unresolved-variable", 8, false, false)
        };
        let rendered = rendered_for(vec![failing], true);

        for fragment in [
            "k9s-leaves-no-unresolved-variable",
            "expect.stdout.absent[0]",
            "ZSH_ENV_DIR",
            "0/8",
        ] {
            assert!(
                rendered.contains(fragment),
                "a failure must name the case, the expectation and the value it got, or \
                 the reader has to open gaveldrop's code. Missing {fragment:?} in:\n{rendered}"
            );
        }
    }

    #[test]
    fn an_unexpected_call_is_reported_even_with_no_diffs() {
        let failing = Outcome {
            unexpected_calls: vec!["kubectl".to_string()],
            ..outcome("t", 5, false, false)
        };
        assert!(rendered_for(vec![failing], false).contains("kubectl"));
    }

    #[test]
    fn a_tee_feeds_every_renderer_it_was_given() {
        let mut first = Vec::new();
        let mut second = Vec::new();
        let report = Report::from(vec![outcome("shared", 5, true, false)]);

        {
            let mut tee = Tee::new();
            tee.add(Box::new(terminal::Terminal::plain(&mut first)));
            tee.add(Box::new(jsonl::Jsonl::new(&mut second)));

            for outcome in &report.outcomes {
                tee.case_finished(outcome);
            }
            tee.finish(&report);
        }

        assert!(
            String::from_utf8_lossy(&first).contains("shared"),
            "the terminal must still see every case"
        );
        assert!(
            String::from_utf8_lossy(&second).contains("\"name\":\"shared\""),
            "and so must the machine-readable file, while the suite is still running"
        );
    }

    #[test]
    fn an_empty_tee_is_harmless() {
        let report = Report::from(vec![outcome("a", 1, true, false)]);
        let mut tee = Tee::new();

        tee.case_finished(&report.outcomes[0]);
        tee.finish(&report);
    }

    #[test]
    fn a_tolerated_failure_reads_differently_from_a_real_one() {
        let tolerated = rendered_for(vec![outcome("known", 3, false, true)], false);
        let failed = rendered_for(vec![outcome("broken", 3, false, false)], false);

        assert_ne!(
            tolerated.split_whitespace().next(),
            failed.split_whitespace().next(),
            "a tolerated failure and a real one must not look identical, or `allow_fail` \
             becomes a way to hide things rather than to declare them"
        );
    }
}
