//! Aggregating outcomes, and rendering them.

pub mod annotate;
pub mod badge;
pub mod html;
pub mod jsonl;
pub mod junit;
pub mod lines;
pub mod merge;
pub mod teamcity;
pub mod terminal;
pub mod verbose;

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
    /// How long every case took together, in milliseconds.
    ///
    /// A sum of the cases, not a wall clock: it excludes discovery and whatever the report itself
    /// costs, so it answers "where did the time go" rather than "how long did I wait". The two are
    /// close today because cases run one after another.
    #[serde(default)]
    pub duration_ms: u64,
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
            duration_ms: 0,
        };

        for outcome in &self.outcomes {
            summary.max_score = summary.max_score.saturating_add(outcome.weight);
            summary.duration_ms = summary.duration_ms.saturating_add(outcome.duration_ms);
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

    /// Whether this run cleared `gate`, and every reason it did not.
    ///
    /// **Every** reason, not the first: fixing one threshold to discover another is two runs where
    /// one would do.
    pub fn gate(&self, gate: &crate::config::GateConfig) -> Gating {
        let summary = self.summary();
        let mut reasons = Vec::new();

        if let Some(least) = gate.min_score {
            if least > summary.max_score {
                // A threshold above the suite's own total can never be met, so every run fails and
                // the honest message above reads as a suite problem. It is almost always the same
                // mistake — `min_score: 80` written for "80 %" — and saying so is the difference
                // between one puzzled run and a lost afternoon.
                reasons.push(format!(
                    "gate.min_score is {least} and the whole suite is worth {}, so this threshold \
                     can never be met. It is a weighted total, not a percentage: add up the \
                     `weight:` of your cases to choose it",
                    summary.max_score
                ));
            } else if summary.score < least {
                reasons.push(format!(
                    "the weighted score is {} of {}, below the {least} this project requires",
                    summary.score, summary.max_score
                ));
            }
        }

        if let Some(most) = gate.max_tolerated
            && summary.tolerated > most
        {
            reasons.push(format!(
                "{} tolerated failures, and this project allows {most}",
                summary.tolerated
            ));
        }

        if let Some(above) = gate.fail_above_weight {
            for outcome in &self.outcomes {
                if !outcome.passed && !outcome.allow_fail && outcome.weight > above {
                    reasons.push(format!(
                        "`{}` failed, and its weight of {} is above the {above} this project \
                         treats as unconditional",
                        outcome.name, outcome.weight
                    ));
                }
            }
        }

        Gating {
            passed: reasons.is_empty(),
            reasons,
        }
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

/// Whether a run cleared the project's thresholds, and why not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gating {
    /// Whether every enforced threshold held.
    pub passed: bool,
    /// One sentence per threshold that did not, in the order they are declared.
    pub reasons: Vec<String>,
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
    /// What the engine decided before invoking a case, for whoever is diagnosing one that does
    /// not do what they expect.
    ///
    /// Empty by default, so every renderer that has no use for it is unchanged. It is **not** an
    /// observation and does not belong in one: an observation records what the subject produced,
    /// and this records what we did to it.
    ///
    /// Called once per case with the whole note, lines already separated. A renderer that shows it
    /// prints; the JSON, HTML and JUnit ones ignore it, because a trace of the engine is not a
    /// verdict and a dashboard has no column for it.
    fn preparing(&mut self, _case: &str, _note: &[String]) {}
    /// What the subject produced, before the verdict on it.
    ///
    /// Empty by default, like `preparing`, so no existing renderer changes and a consumer's own sink
    /// keeps compiling. The terminal one ignores it: a run streaming every subject's output would
    /// bury the verdicts it exists to show. The HTML report uses it, because a page has room for
    /// what a line does not.
    ///
    /// Not called for a case that never ran — a broken document, an adapter that claimed nothing.
    /// There is nothing to report there, and inventing an empty observation would say the subject
    /// wrote nothing rather than that it never started.
    fn observed(&mut self, _case: &str, _observations: &crate::Observations) {}
    /// The exchanges the case declared, in order, each with the name it gave itself.
    ///
    /// **What the case said, not what happened** — so a renderer can draw a step that passed. A step's
    /// name reaches a report today only through the path of a diff it produced, which means a passing
    /// exchange is nameless and an outcome alone cannot describe the shape of the case it came from.
    ///
    /// Defaulted like the two above: a renderer showing a flat list of cases needs none of it, and a
    /// consumer's own sink keeps compiling. Called before the subject is invoked, so a renderer that
    /// nests has the shape before it has any verdict.
    ///
    /// Not called for a case with no `steps:`. An empty list and no list are the same thing to every
    /// reader, and sending one would make every single-exchange case look like it declared something.
    fn declares_steps(&mut self, _case: &str, _names: &[Option<String>]) {}
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

    fn preparing(&mut self, case: &str, note: &[String]) {
        for sink in &mut self.sinks {
            sink.preparing(case, note);
        }
    }

    fn observed(&mut self, case: &str, observations: &crate::Observations) {
        for sink in &mut self.sinks {
            sink.observed(case, observations);
        }
    }

    fn declares_steps(&mut self, case: &str, names: &[Option<String>]) {
        for sink in &mut self.sinks {
            sink.declares_steps(case, names);
        }
    }
}

/// A duration in the shortest form that stays honest, such as `312ms` or `1.2s`.
///
/// Two units rather than one, because the two questions are different. Under a second, the reader is
/// scanning for the case that stands out and wants to compare `4ms` against `700ms` without counting
/// zeroes. Above a second, the third digit is noise: `1.2s` and `1247ms` say the same thing and only
/// one of them reads at a glance.
///
/// Milliseconds are the stored unit everywhere, so this is presentation only — nothing parses it
/// back, and the exact number stays in the JSON Lines report for anything that wants to do
/// arithmetic.
pub fn duration(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else {
        format!("{}.{}s", ms / 1_000, (ms % 1_000) / 100)
    }
}

/// A duration as decimal seconds, which is what JUnit's `time=` means.
///
/// Its own function rather than a variant of [`duration`]: that one is for a human reading a
/// terminal, this one is a wire format a CI reads, and letting one drift into the other is how
/// `1.2s` ends up inside an XML attribute that expects a number.
pub fn seconds(ms: u64) -> String {
    format!("{}.{:03}", ms / 1_000, ms % 1_000)
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
            duration_ms: 0,
        }
    }

    /// Passing cases of equal weight, distinguished only by how long they took.
    fn timed(cases: &[(&str, u64)]) -> Vec<Outcome> {
        cases
            .iter()
            .map(|(name, ms)| Outcome {
                duration_ms: *ms,
                ..outcome(name, 1, true, false)
            })
            .collect()
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

    fn gated(outcomes: Vec<Outcome>, gate: &crate::config::GateConfig) -> Gating {
        Report::from(outcomes).gate(gate)
    }

    fn gate(
        min_score: Option<u32>,
        max_tolerated: Option<usize>,
        above: Option<u32>,
    ) -> crate::config::GateConfig {
        crate::config::GateConfig {
            min_score,
            max_tolerated,
            fail_above_weight: above,
        }
    }

    #[test]
    fn an_absent_gate_passes_everything() {
        let verdict = gated(
            vec![outcome("broken", 8, false, false)],
            &crate::config::GateConfig::default(),
        );

        assert!(
            verdict.passed,
            "gating is opt-in: adding it must not start failing projects that never asked for a \
             threshold. A failing case still fails the run on its own — that is `is_success`, not \
             the gate"
        );
    }

    #[test]
    fn a_score_below_the_minimum_fails_and_says_by_how_much() {
        let verdict = gated(
            vec![outcome("a", 5, true, false), outcome("b", 5, false, false)],
            &gate(Some(8), None, None),
        );

        assert!(!verdict.passed);
        let said = verdict.reasons.join(" ");
        assert!(
            said.contains('5') && said.contains('8'),
            "both numbers, or the reader has to compute the shortfall from a report they cannot \
             see: {said}"
        );
    }

    #[test]
    fn a_threshold_above_the_suites_own_total_says_it_can_never_be_met() {
        // The mistake a consumer actually made: `min_score: 80` copied from a document, read as
        // "80 %", against a suite whose weights add up to 10. Every run failed, and the honest
        // message — "the weighted score is 10 of 10, below the 80 this project requires" — reads as
        // a problem with the suite rather than with the threshold.
        let verdict = gated(
            vec![outcome("a", 5, true, false), outcome("b", 5, true, false)],
            &gate(Some(80), None, None),
        );

        assert!(!verdict.passed, "the gate still fails, which is right");
        let said = verdict.reasons.join(" ");
        assert!(
            said.contains("never be met"),
            "an unreachable threshold is a configuration mistake, not a suite failure, and the two \
             deserve different sentences: {said}"
        );
        assert!(
            said.contains("not a percentage"),
            "and the mistake is nearly always that one, so naming it saves the afternoon: {said}"
        );
    }

    #[test]
    fn a_reachable_threshold_that_is_missed_still_reports_the_shortfall() {
        let verdict = gated(
            vec![outcome("a", 5, true, false), outcome("b", 5, false, false)],
            &gate(Some(8), None, None),
        );

        let said = verdict.reasons.join(" ");
        assert!(
            !said.contains("never be met"),
            "8 of 10 is perfectly reachable; only the unreachable case gets the other sentence: \
             {said}"
        );
    }

    #[test]
    fn more_tolerated_failures_than_allowed_fails_the_gate() {
        let verdict = gated(
            vec![
                outcome("known-one", 3, false, true),
                outcome("known-two", 3, false, true),
            ],
            &gate(None, Some(1), None),
        );

        assert!(
            !verdict.passed,
            "`allow_fail` is an exemption, and an exemption nobody counts becomes a habit"
        );
    }

    #[test]
    fn a_heavy_case_failing_fails_the_gate_whatever_the_score() {
        let verdict = gated(
            vec![
                outcome("small", 1, false, false),
                outcome("critical", 9, false, false),
                outcome("rest", 90, true, false),
            ],
            &gate(Some(50), None, Some(8)),
        );

        assert!(
            !verdict.passed,
            "ninety percent of the weight holding is no comfort when the case that broke is the \
             one that mattered"
        );
        assert!(
            verdict.reasons.iter().any(|why| why.contains("critical")),
            "and it must name which case, not just that one was heavy: {:?}",
            verdict.reasons
        );
    }

    #[test]
    fn a_heavy_case_that_passes_does_not_fail_the_gate() {
        let verdict = gated(
            vec![outcome("critical", 9, true, false)],
            &gate(None, None, Some(8)),
        );

        assert!(verdict.passed, "{:?}", verdict.reasons);
    }

    #[test]
    fn a_tolerated_heavy_failure_does_not_trip_the_weight_rule() {
        let verdict = gated(
            vec![outcome("known-heavy", 9, false, true)],
            &gate(None, None, Some(8)),
        );

        assert!(
            verdict.passed,
            "a declared exemption is not a surprise. `max_tolerated` is the knob that counts those, \
             and having both fire on one case would make the two rules impossible to reason about: \
             {:?}",
            verdict.reasons
        );
    }

    #[test]
    fn every_reason_is_reported_not_just_the_first() {
        let verdict = gated(
            vec![
                outcome("known", 3, false, true),
                outcome("critical", 9, false, false),
            ],
            &gate(Some(100), Some(0), Some(8)),
        );

        assert_eq!(
            verdict.reasons.len(),
            3,
            "fixing one threshold to discover another is two runs where one would do: {:?}",
            verdict.reasons
        );
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

    /// The two units, and the boundary between them.
    ///
    /// A table rather than one case per assertion, because what matters here is that no input
    /// produces something unreadable — `1000ms`, `0.9s` or `1.24999s` would each be a small
    /// embarrassment in a report someone attaches to a pull request.
    #[test]
    fn a_duration_reads_in_the_unit_that_suits_it() {
        for (ms, expected) in [
            (0, "0ms"),
            (7, "7ms"),
            (312, "312ms"),
            (999, "999ms"),
            (1_000, "1.0s"),
            (1_249, "1.2s"),
            (12_345, "12.3s"),
        ] {
            assert_eq!(duration(ms), expected, "for {ms}ms");
        }
    }

    /// JUnit's `time` is a number, not a phrase, and the two formatters must not be confused.
    #[test]
    fn seconds_are_decimal_and_never_carry_a_unit() {
        for (ms, expected) in [
            (0, "0.000"),
            (7, "0.007"),
            (312, "0.312"),
            (1_000, "1.000"),
            (12_345, "12.345"),
        ] {
            assert_eq!(seconds(ms), expected, "for {ms}ms");
            assert!(
                seconds(ms).parse::<f64>().is_ok(),
                "a CI parses this as a number: {}",
                seconds(ms)
            );
        }
    }

    /// The summary's duration is the cases', added up.
    ///
    /// Computed rather than stored, like every other field of `Summary` — which is what keeps two
    /// shards mergeable by concatenating their outcomes.
    #[test]
    fn the_summary_adds_up_what_the_cases_took() {
        let mut quick = outcome("quick", 1, true, false);
        quick.duration_ms = 40;
        let mut slow = outcome("slow", 1, false, false);
        slow.duration_ms = 2_000;

        assert_eq!(Report::from(vec![quick, slow]).summary().duration_ms, 2_040);
    }

    /// A fast case says nothing, a slow one says how slow.
    ///
    /// The whole point of a number on a terminal line is that it stands out, and a column of
    /// `2ms` would bury the one line that matters.
    #[test]
    fn the_terminal_names_a_duration_only_when_it_is_worth_naming() {
        let mut quick = outcome("quick", 1, true, false);
        quick.duration_ms = 40;
        let mut slow = outcome("slow", 1, true, false);
        slow.duration_ms = 4_100;

        let text = rendered_for(vec![quick, slow], true);

        assert!(
            text.contains("quick  1/1\n"),
            "a case nobody waited for adds no column: {text}"
        );
        assert!(
            text.contains("slow  1/1  4.1s"),
            "a case somebody waited for says so on its own line: {text}"
        );
        assert!(
            text.contains("· 4.1s"),
            "and the summary always carries the total, however fast the run was: {text}"
        );
    }

    /// The summary names where the time went, in order.
    ///
    /// The case this exists for: on this repository's own suite the slowest case was thirty times
    /// the fastest and still under a second, so every per-case line stayed quiet and the answer was
    /// nowhere. A streaming renderer cannot rank as it goes — only `finish` sees the distribution.
    #[test]
    fn the_summary_names_the_slowest_cases() {
        let text = rendered_for(
            timed(&[("quick", 10), ("slow", 520), ("middling", 270)]),
            true,
        );

        let line = text
            .lines()
            .find(|line| line.starts_with("slowest"))
            .unwrap_or_else(|| panic!("no ranking in:\n{text}"));

        assert!(
            line.starts_with("slowest — slow 520ms · middling 270ms · quick 10ms"),
            "slowest first, or the reader has to sort three numbers themselves: {line}"
        );
    }

    /// A suite where nothing is slow says nothing about it.
    #[test]
    fn a_uniformly_fast_run_is_not_ranked() {
        let text = rendered_for(timed(&[("a", 8), ("b", 11), ("c", 20)]), true);

        assert!(
            !text.contains("slowest"),
            "naming the three slowest of a 20ms suite costs attention and buys nothing: {text}"
        );
    }

    /// Two cases are not a ranking.
    #[test]
    fn a_run_too_small_to_rank_is_not_ranked() {
        let text = rendered_for(timed(&[("a", 400), ("b", 900)]), true);

        assert!(
            !text.contains("slowest"),
            "with two cases the two lines above already say it: {text}"
        );
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
