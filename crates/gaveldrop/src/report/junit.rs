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

use crate::report::{Report, Sink, failure_lines};
use crate::{Diff, Outcome};

/// Collects outcomes and writes JUnit XML once the run is over.
pub struct Junit<W: Write> {
    out: W,
    outcomes: Vec<Outcome>,
}

impl<W: Write> Junit<W> {
    /// A renderer writing into `out`.
    pub fn new(out: W) -> Self {
        Self {
            out,
            outcomes: Vec::new(),
        }
    }
}

impl<W: Write> Sink for Junit<W> {
    fn case_finished(&mut self, outcome: &Outcome) {
        self.outcomes.push(outcome.clone());
    }

    fn finish(&mut self, report: &Report) {
        let summary = report.summary();
        let _ = writeln!(self.out, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
        let _ = writeln!(
            self.out,
            r#"<testsuites tests="{}" failures="{}" skipped="{}">"#,
            summary.total, summary.failed, summary.tolerated
        );
        let _ = writeln!(
            self.out,
            r#"  <testsuite name="gaveldrop" tests="{}" failures="{}" skipped="{}">"#,
            summary.total, summary.failed, summary.tolerated
        );

        for outcome in &self.outcomes {
            write_case(&mut self.out, outcome);
        }

        let _ = writeln!(self.out, "  </testsuite>");
        let _ = writeln!(self.out, "</testsuites>");
        let _ = self.out.flush();
    }
}

/// One `<testcase>`, with a child only when there is something to say.
///
/// A tolerated failure becomes `<skipped>` rather than `<failure>`. It is the closest honest mapping:
/// reporting it as a failure would break the build that a tolerance exists to keep green, and
/// reporting it as a plain pass would hide a known defect the project deliberately wrote down.
fn write_case<W: Write>(out: &mut W, outcome: &Outcome) {
    let name = escaped(&outcome.name);

    if outcome.passed {
        let _ = writeln!(
            out,
            r#"    <testcase name="{name}" classname="gaveldrop"/>"#
        );
        return;
    }

    let _ = writeln!(out, r#"    <testcase name="{name}" classname="gaveldrop">"#);

    let detail = escaped(&failure_lines(outcome).join("\n"));
    let headline = escaped(&summarise(&outcome.diffs));

    if outcome.allow_fail {
        let _ = writeln!(out, r#"      <skipped message="tolerated: {headline}"/>"#);
    } else {
        let _ = writeln!(out, r#"      <failure message="{headline}">"#);
        let _ = writeln!(out, "{detail}");
        let _ = writeln!(out, "      </failure>");
    }

    let _ = writeln!(out, "    </testcase>");
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
            xml.contains(r#"<testcase name="fine" classname="gaveldrop"/>"#),
            "a passing case needs no body, and a self-closing element is what every dashboard \
             expects: {xml}"
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
