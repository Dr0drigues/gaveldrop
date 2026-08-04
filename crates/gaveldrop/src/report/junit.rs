//! The JUnit XML report.
//!
//! Written by hand. A serialisation crate for a format with four element types would be a dependency
//! to justify for no gain, and the escaping is five characters plus a range of bytes no XML document
//! may carry.
//!
//! **This renderer is the one that cannot stream.** Its header carries the totals, so nothing can be
//! written until the last case has finished — unlike the terminal and JSON Lines sinks, which emit as
//! they go. That is a property of the format rather than a choice, and it is why `Sink` has both
//! `case_finished` and `finish`.

use std::io::Write;

use crate::report::{Report, Sink, seconds};
use crate::{Diff, Outcome};

/// Collects outcomes and writes JUnit XML once the run is over.
pub struct Junit<W: Write> {
    out: W,
    outcomes: Vec<Outcome>,
    declared: std::collections::BTreeMap<String, Vec<Option<String>>>,
}

impl<W: Write> Junit<W> {
    /// A renderer writing into `out`.
    pub fn new(out: W) -> Self {
        Self {
            out,
            outcomes: Vec::new(),
            declared: std::collections::BTreeMap::new(),
        }
    }
}

impl<W: Write> Sink for Junit<W> {
    fn case_finished(&mut self, outcome: &Outcome) {
        self.outcomes.push(outcome.clone());
    }

    fn declares_steps(&mut self, case: &str, names: &[Option<String>]) {
        self.declared.insert(case.to_string(), names.to_vec());
    }

    fn finish(&mut self, report: &Report) {
        // Built before anything is written, because the counts on the element have to match the elements
        // inside it. A case that declared exchanges contributes several, so `summary.total` is the number
        // of cases and no longer the number of tests.
        let entries: Vec<Entry> = self
            .outcomes
            .iter()
            .flat_map(|outcome| expanded(outcome, self.declared.get(&outcome.name)))
            .collect();

        let failures = entries
            .iter()
            .filter(|entry| entry.failed && !entry.tolerated)
            .count();
        let skipped = entries.iter().filter(|entry| entry.tolerated).count();
        let summary = report.summary();

        let _ = writeln!(self.out, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
        let _ = writeln!(
            self.out,
            r#"<testsuites tests="{}" failures="{failures}" skipped="{skipped}">"#,
            entries.len()
        );
        let _ = writeln!(
            self.out,
            r#"  <testsuite name="gaveldrop" tests="{}" failures="{failures}" skipped="{skipped}" time="{}">"#,
            entries.len(),
            seconds(summary.duration_ms)
        );

        for entry in &entries {
            write_entry(&mut self.out, entry);
        }

        let _ = writeln!(self.out, "  </testsuite>");
        let _ = writeln!(self.out, "</testsuites>");
        let _ = self.out.flush();
    }
}

/// One `<testcase>` to write: a whole case, or one exchange of it.
struct Entry {
    /// `gaveldrop`, or `gaveldrop.<case>` for an exchange of a case that declared several.
    classname: String,
    name: String,
    /// Absent where nothing was measured, which is every exchange.
    duration_ms: Option<u64>,
    diffs: Vec<Diff>,
    failed: bool,
    tolerated: bool,
}

/// The entries one outcome contributes.
///
/// **`classname`, not a nested `<testsuite>`.** JUnit's `classname` is exactly "which suite this test
/// belongs to" and every dashboard groups by it, where nesting suite elements is understood by some
/// parsers and quietly flattened by others. So a case that declared exchanges becomes
/// `gaveldrop.<case>` holding one test per exchange plus one for the run as a whole — the same shape the
/// test tree draws, in the vocabulary this format already has.
fn expanded(outcome: &Outcome, steps: Option<&Vec<Option<String>>>) -> Vec<Entry> {
    let Some(steps) = steps else {
        return vec![Entry {
            classname: "gaveldrop".to_string(),
            name: outcome.name.clone(),
            duration_ms: Some(outcome.duration_ms),
            diffs: outcome.diffs.clone(),
            failed: !outcome.passed,
            tolerated: !outcome.passed && outcome.allow_fail,
        }];
    };

    let classname = format!("gaveldrop.{}", outcome.name);
    let mut entries = vec![Entry {
        classname: classname.clone(),
        name: WHOLE_RUN.to_string(),
        duration_ms: Some(outcome.duration_ms),
        diffs: sifted(outcome, None),
        failed: false,
        tolerated: false,
    }];

    for (index, declared) in steps.iter().enumerate() {
        entries.push(Entry {
            classname: classname.clone(),
            name: match declared {
                Some(given) => given.clone(),
                // Only the whole run is timed: a case is measured and an exchange is not, and a
                // fabricated `time="0"` would put every exchange at the top of a slowest-tests list.
                None => format!("step {}", index + 1),
            },
            duration_ms: None,
            diffs: sifted(outcome, Some(index)),
            failed: false,
            tolerated: false,
        });
    }

    for entry in &mut entries {
        entry.failed = !entry.diffs.is_empty();
        entry.tolerated = entry.failed && outcome.allow_fail;
    }

    entries
}

/// The name of the entry carrying what the case asserted across its exchanges.
const WHOLE_RUN: &str = "the run as a whole";

/// The diffs belonging to one exchange, or to everything that is not one.
fn sifted(outcome: &Outcome, step: Option<usize>) -> Vec<Diff> {
    outcome
        .diffs
        .iter()
        .filter(|diff| match step {
            Some(index) => diff.path.starts_with(&format!("steps[{index}]")),
            None => !diff.path.starts_with("steps["),
        })
        .cloned()
        .collect()
}

/// One `<testcase>`, with a child only when there is something to say.
///
/// A tolerated failure becomes `<skipped>` rather than `<failure>`. It is the closest honest mapping:
/// reporting it as a failure would break the build that a tolerance exists to keep green, and
/// reporting it as a plain pass would hide a known defect the project deliberately wrote down.
fn write_entry<W: Write>(out: &mut W, entry: &Entry) {
    let name = escaped(&entry.name);
    let classname = escaped(&entry.classname);

    // `time` is what every CI dashboard reads to draw its slowest-tests list, and it is the one
    // place a duration is not decoration: a JUnit file without it makes that feature silently show
    // zeroes rather than say it has no data. Omitted entirely where nothing was measured, for the same
    // reason — an attribute of `0` is a measurement and an absent one is not.
    let time = match entry.duration_ms {
        Some(ms) => format!(r#" time="{}""#, seconds(ms)),
        None => String::new(),
    };

    if !entry.failed {
        let _ = writeln!(
            out,
            r#"    <testcase name="{name}" classname="{classname}"{time}/>"#
        );
        return;
    }

    let _ = writeln!(
        out,
        r#"    <testcase name="{name}" classname="{classname}"{time}>"#
    );

    let detail = escaped(
        &entry
            .diffs
            .iter()
            .map(described)
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let headline = escaped(&summarise(&entry.diffs));

    if entry.tolerated {
        let _ = writeln!(out, r#"      <skipped message="tolerated: {headline}"/>"#);
    } else {
        let _ = writeln!(out, r#"      <failure message="{headline}">"#);
        let _ = writeln!(out, "{detail}");
        let _ = writeln!(out, "      </failure>");
    }

    let _ = writeln!(out, "    </testcase>");
}

/// One diff as the line a reader reads, the same wording every renderer uses.
fn described(diff: &Diff) -> String {
    format!(
        "    {}\n      expected  {}\n      got       {}",
        diff.path, diff.expected, diff.got
    )
}

/// The one-line summary a dashboard shows before anyone opens the detail.
///
/// The first assertion path, because a dashboard that lists twenty cases as "failed" with no
/// distinction is a list nobody reads. The rest is in the body.
fn summarise(diffs: &[Diff]) -> String {
    match diffs.split_first() {
        None => "an unexpected call reached the catch-all".to_string(),
        Some((first, [])) => first.path.clone(),
        Some((first, rest)) => format!("{} and {} more", first.path, rest.len()),
    }
}

/// Text safe to put inside an XML document, in an attribute or in an element.
///
/// All five characters, in every position. Escaping the name but not the value is the usual shape of
/// this bug, and a subject that printed `<html>` would produce a document no CI dashboard can parse.
///
/// Control bytes are dropped rather than escaped: XML 1.0 has no representation for most of them, so
/// `&#1;` would be just as invalid as the byte itself. A subject that emitted one loses it from the
/// report and keeps it in the terminal output, which is the lesser harm.
fn escaped(text: &str) -> String {
    text.chars()
        .filter(|c| !is_forbidden(*c))
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            other => other.to_string(),
        })
        .collect()
}

/// Whether XML 1.0 forbids this character outright.
fn is_forbidden(c: char) -> bool {
    let code = c as u32;
    (code < 0x20 && c != '\t' && c != '\n' && c != '\r') || code == 0xFFFE || code == 0xFFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(name: &str, passed: bool, allow_fail: bool, diffs: Vec<Diff>) -> Outcome {
        Outcome {
            name: name.to_string(),
            weight: 5,
            allow_fail,
            passed,
            diffs,
            unexpected_calls: Vec::new(),
            unmentioned_files: Vec::new(),
            duration_ms: 0,
        }
    }

    fn diff(path: &str, got: &str) -> Diff {
        Diff {
            path: path.to_string(),
            expected: "something".to_string(),
            got: got.to_string(),
        }
    }

    fn rendered(outcomes: Vec<Outcome>) -> String {
        let report = Report::from(outcomes);
        let mut buffer = Vec::new();
        {
            let mut sink = Junit::new(&mut buffer);
            for outcome in &report.outcomes {
                sink.case_finished(outcome);
            }
            sink.finish(&report);
        }
        String::from_utf8_lossy(&buffer).into_owned()
    }

    #[test]
    fn a_passing_case_is_a_testcase_with_no_children() {
        let xml = rendered(vec![outcome("fine", true, false, Vec::new())]);

        assert!(
            xml.contains(r#"<testcase name="fine" classname="gaveldrop" time="0.000"/>"#),
            "a passing case needs no body, and a self-closing element is what every dashboard \
             expects: {xml}"
        );
    }

    /// A case that declared exchanges groups them under a `classname`.
    ///
    /// **`classname`, not a nested `<testsuite>`.** JUnit's `classname` is exactly "which suite this test
    /// belongs to" and every dashboard groups by it, where nesting suite elements is understood by some
    /// parsers and quietly flattened by others. This is the same shape the test tree draws, said in the
    /// vocabulary the format already has.
    #[test]
    fn a_case_with_exchanges_groups_them_under_its_own_classname() {
        let passing = outcome("ordering", true, false, Vec::new());
        let mut buffer = Vec::new();
        {
            let mut sink = Junit::new(&mut buffer);
            sink.declares_steps("ordering", &[Some("creates an order".to_string()), None]);
            sink.case_finished(&passing);
            sink.finish(&Report::from(vec![passing]));
        }
        let xml = String::from_utf8_lossy(&buffer).into_owned();

        assert!(
            xml.contains(r#"name="the run as a whole" classname="gaveldrop.ordering""#),
            "{xml}"
        );
        assert!(
            xml.contains(r#"name="creates an order" classname="gaveldrop.ordering""#),
            "{xml}"
        );
        assert!(
            xml.contains(r#"name="step 2" classname="gaveldrop.ordering""#),
            "{xml}"
        );
    }

    /// The counts on the element match the elements inside it.
    ///
    /// A case that declared exchanges contributes several tests, so `summary.total` is the number of
    /// cases and no longer the number of tests. A file whose attributes disagreed with its contents is
    /// the kind of thing a dashboard reports as a mystery rather than as a malformed file.
    #[test]
    fn the_counts_match_what_was_written() {
        let plain = outcome("plain", true, false, Vec::new());
        let stepped = outcome(
            "ordering",
            false,
            false,
            vec![diff("steps[0] \"only\".exit_code", "4")],
        );

        let mut buffer = Vec::new();
        {
            let mut sink = Junit::new(&mut buffer);
            sink.declares_steps("ordering", &[Some("only".to_string())]);
            sink.case_finished(&plain);
            sink.case_finished(&stepped);
            sink.finish(&Report::from(vec![plain, stepped]));
        }
        let xml = String::from_utf8_lossy(&buffer).into_owned();

        assert_eq!(
            xml.matches("<testcase ").count(),
            3,
            "one plain case, plus a whole run and one exchange: {xml}"
        );
        assert!(xml.contains(r#"tests="3" failures="1""#), "{xml}");
    }

    /// Each failure lands on the entry it came from, and the exchange that held is left alone.
    #[test]
    fn a_failure_lands_on_the_entry_it_came_from() {
        let failing = outcome(
            "ordering",
            false,
            false,
            vec![
                diff("expect.exit_code", "0"),
                diff("steps[1] \"the second\".exit_code", "4"),
            ],
        );

        let mut buffer = Vec::new();
        {
            let mut sink = Junit::new(&mut buffer);
            sink.declares_steps(
                "ordering",
                &[
                    Some("the first".to_string()),
                    Some("the second".to_string()),
                ],
            );
            sink.case_finished(&failing);
            sink.finish(&Report::from(vec![failing]));
        }
        let xml = String::from_utf8_lossy(&buffer).into_owned();

        assert!(
            xml.contains(r#"name="the first" classname="gaveldrop.ordering"/>"#),
            "the exchange that held is self-closing: {xml}"
        );
        assert_eq!(xml.matches("<failure").count(), 2, "{xml}");
        assert!(xml.contains(r#"tests="3" failures="2""#), "{xml}");
    }

    /// An exchange carries no `time`, because nothing measured one.
    ///
    /// An attribute of `0` is a measurement and an absent one is not — and every exchange claiming zero
    /// would sit at the top of a dashboard's slowest-tests list.
    #[test]
    fn an_exchange_carries_no_time_attribute() {
        let passing = outcome("ordering", true, false, Vec::new());
        let mut buffer = Vec::new();
        {
            let mut sink = Junit::new(&mut buffer);
            sink.declares_steps("ordering", &[None]);
            sink.case_finished(&passing);
            sink.finish(&Report::from(vec![passing]));
        }
        let xml = String::from_utf8_lossy(&buffer).into_owned();

        assert!(
            xml.contains(r#"name="step 1" classname="gaveldrop.ordering"/>"#),
            "{xml}"
        );
        assert_eq!(
            xml.matches(" time=").count(),
            2,
            "the suite and the whole run, and nothing else: {xml}"
        );
    }

    /// The attribute a dashboard reads to draw its slowest-tests list.
    ///
    /// Asserted on a failing case as well as a passing one, because the two are written by
    /// different branches: the failing branch opens the element to nest a `<failure>` in it, and
    /// forgetting the attribute on exactly one of the two is how half a dashboard's timings go
    /// missing.
    #[test]
    fn every_testcase_carries_the_time_it_took() {
        let mut fast = outcome("fast", true, false, Vec::new());
        fast.duration_ms = 312;
        let mut slow = outcome("slow", false, false, vec![diff("expect.exit_code", "1")]);
        slow.duration_ms = 12_345;

        let xml = rendered(vec![fast, slow]);

        assert!(
            xml.contains(r#"<testcase name="fast" classname="gaveldrop" time="0.312"/>"#),
            "decimal seconds is what the format means by time: {xml}"
        );
        assert!(
            xml.contains(r#"<testcase name="slow" classname="gaveldrop" time="12.345">"#),
            "a failing case is timed too: {xml}"
        );
        assert!(
            xml.contains(r#"time="12.657""#),
            "the suite's own time is the sum of its cases: {xml}"
        );
    }

    #[test]
    fn a_failing_case_carries_its_diffs_in_the_failure_body() {
        let xml = rendered(vec![outcome(
            "broken",
            false,
            false,
            vec![diff("expect.stdout.contains[0]", "nothing of the sort")],
        )]);

        for fragment in [
            r#"<failure message="expect.stdout.contains[0]">"#,
            "expect.stdout.contains[0]",
            "nothing of the sort",
        ] {
            assert!(
                xml.contains(fragment),
                "the path, what was wanted and what came back — the same three things every other \
                 renderer shows, so nobody learns a second vocabulary. Missing {fragment:?} in \
                 {xml}"
            );
        }
    }

    #[test]
    fn a_tolerated_failure_is_skipped_rather_than_failed() {
        let xml = rendered(vec![outcome(
            "known",
            false,
            true,
            vec![diff("expect.exit_code", "7")],
        )]);

        assert!(
            xml.contains("<skipped") && !xml.contains("<failure"),
            "reporting it as a failure would break the build a tolerance exists to keep green; \
             reporting it as a plain pass would hide a defect the project deliberately wrote \
             down: {xml}"
        );
        assert!(
            xml.contains("tolerated"),
            "and it must say so, or a skipped count is a mystery: {xml}"
        );
    }

    #[test]
    fn the_counts_in_the_header_match_the_outcomes() {
        let xml = rendered(vec![
            outcome("a", true, false, Vec::new()),
            outcome("b", false, false, vec![diff("expect.exit_code", "1")]),
            outcome("c", false, true, vec![diff("expect.exit_code", "2")]),
        ]);

        assert!(
            xml.contains(r#"<testsuites tests="3" failures="1" skipped="1">"#),
            "a tolerated failure counts as skipped and not as a failure, exactly as the weighted \
             summary already counts it: {xml}"
        );
    }

    #[test]
    fn the_five_xml_characters_are_escaped_wherever_they_appear() {
        let xml = rendered(vec![outcome(
            "a <case> & \"friends\"",
            false,
            false,
            vec![diff("expect.stdout", "<html>it's 5 > 3</html>")],
        )]);

        assert!(
            !xml.contains("<html>") && !xml.contains("<case>"),
            "escaped in the name but not in the value is the usual shape of this bug, and a \
             subject that printed markup must not produce a document nothing can parse: {xml}"
        );
        for entity in ["&lt;", "&gt;", "&amp;", "&quot;", "&apos;"] {
            assert!(xml.contains(entity), "missing {entity} in {xml}");
        }
    }

    #[test]
    fn a_control_byte_is_dropped_rather_than_escaped() {
        let xml = rendered(vec![outcome(
            "noisy",
            false,
            false,
            vec![diff("expect.stdout", "before\u{1}after")],
        )]);

        assert!(
            xml.contains("beforeafter") && !xml.contains('\u{1}'),
            "XML 1.0 has no representation for most control bytes, so `&#1;` would be just as \
             invalid as the byte. Dropping it keeps the document parseable and the byte is still \
             in the terminal output: {xml}"
        );
    }

    #[test]
    fn a_failure_with_several_diffs_says_how_many_in_its_headline() {
        let xml = rendered(vec![outcome(
            "several",
            false,
            false,
            vec![diff("expect.status", "500"), diff("expect.body", "empty")],
        )]);

        assert!(
            xml.contains("and 1 more"),
            "a dashboard listing twenty cases as `failed` with no distinction is a list nobody \
             reads: {xml}"
        );
    }

    #[test]
    fn an_empty_run_is_a_valid_document_rather_than_nothing() {
        let xml = rendered(Vec::new());

        assert!(xml.contains("<testsuites tests=\"0\""));
        assert!(
            xml.contains("</testsuites>"),
            "a truncated document fails a CI parser in a way that looks like our bug rather than \
             an empty suite: {xml}"
        );
    }
}
