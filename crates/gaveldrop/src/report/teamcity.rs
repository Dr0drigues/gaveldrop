//! Failures as TeamCity service messages, which is how a JetBrains IDE draws a test tree.
//!
//! **The protocol, not a plugin.** An IntelliJ-based IDE builds its test tree by parsing
//! `##teamcity[…]` lines out of a process's standard output — the same lines TeamCity itself reads. So
//! the half of an IDE integration that belongs in this repository is a renderer, in Rust, next to the
//! other six. What still needs a plugin is the wiring that attaches the IDE's test console to a run
//! configuration; this is what that plugin would consume, and what TeamCity consumes with no plugin at
//! all.
//!
//! Streamed per case rather than assembled at the end, unlike JUnit: an IDE draws each case as it
//! finishes, and a tree that appeared complete only after the last case would lose most of its point.

use std::io::Write;

use crate::report::{Report, Sink, failure_lines};
use crate::{Diff, Outcome};

/// Writes service messages as each case finishes.
pub struct TeamCity<W: Write> {
    out: W,
    opened: bool,
}

/// The suite name the tree is rooted at.
const SUITE: &str = "gaveldrop";

impl<W: Write> TeamCity<W> {
    /// A renderer writing into `out`, which has to be the standard output an IDE is reading.
    pub fn new(out: W) -> Self {
        Self { out, opened: false }
    }

    /// Opens the suite on the first case, so nothing is emitted for a run that never starts one.
    fn open(&mut self) {
        if !self.opened {
            self.opened = true;
            let _ = writeln!(self.out, "##teamcity[testSuiteStarted name='{SUITE}']");
        }
    }
}

impl<W: Write> Sink for TeamCity<W> {
    fn case_finished(&mut self, outcome: &Outcome) {
        self.open();
        let name = escaped(&outcome.name);

        let _ = writeln!(self.out, "##teamcity[testStarted name='{name}']");

        if !outcome.passed {
            let detail = escaped(&failure_lines(outcome).join("\n"));

            // A tolerated failure is `testIgnored`, the same mapping JUnit gets for the same reason: a
            // failure would break the build the tolerance exists to keep green, and a pass would hide a
            // defect the project deliberately wrote down.
            if outcome.allow_fail {
                let _ = writeln!(
                    self.out,
                    "##teamcity[testIgnored name='{name}' message='tolerated: {}']",
                    escaped(&summarise(&outcome.diffs))
                );
            } else {
                let _ = writeln!(self.out, "{}", failure(&name, outcome, &detail));
            }
        }

        // Milliseconds, which is the unit the message wants and the unit an outcome already keeps.
        let _ = writeln!(
            self.out,
            "##teamcity[testFinished name='{name}' duration='{}']",
            outcome.duration_ms
        );
        let _ = self.out.flush();
    }

    fn finish(&mut self, _report: &Report) {
        // Opened here too, so an empty suite still produces a well-formed pair rather than one
        // unmatched close that an IDE would report as a protocol error.
        self.open();
        let _ = writeln!(self.out, "##teamcity[testSuiteFinished name='{SUITE}']");
        let _ = self.out.flush();
    }
}

/// The failure message, as a comparison whenever there is something to compare.
///
/// **`comparisonFailure` is what earns this renderer its keep.** A `Diff` is already an expectation, a
/// wanted value and a found one, which is exactly the shape the IDE opens its side-by-side viewer for.
/// Every other format has to flatten those three into prose.
///
/// The first diff supplies the comparison and all of them supply the detail, the same division JUnit
/// makes: a dashboard listing twenty failures needs one line each, and the reader who opens one needs
/// everything.
fn failure(name: &str, outcome: &Outcome, detail: &str) -> String {
    match outcome.diffs.first() {
        Some(first) => format!(
            "##teamcity[testFailed type='comparisonFailure' name='{name}' message='{}' \
             details='{detail}' expected='{}' actual='{}']",
            escaped(&first.path),
            escaped(&first.expected),
            escaped(&first.got)
        ),
        // No diffs and still failing means a call reached the catch-all, which compares nothing.
        None => format!(
            "##teamcity[testFailed name='{name}' message='{}' details='{detail}']",
            escaped(&summarise(&outcome.diffs))
        ),
    }
}

/// The one-line summary a collapsed tree node shows.
fn summarise(diffs: &[Diff]) -> String {
    match diffs.split_first() {
        None => "an unexpected call reached the catch-all".to_string(),
        Some((first, [])) => first.path.clone(),
        Some((first, rest)) => format!("{} and {} more", first.path, rest.len()),
    }
}

/// Text safe inside a service message attribute.
///
/// The escape character is a vertical bar, and **it has to be replaced first**: doing it after the
/// others would escape the bars they just introduced, turning every newline into a literal `||n`.
///
/// A control byte becomes a space rather than an escape. The protocol can carry `|0xNNNN`, but a
/// subject that printed a bell into an assertion value is not saying anything a test tree should
/// reproduce, and `verdict::text` already renders control bytes visibly where it matters.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);

    for character in text.chars() {
        match character {
            '|' => out.push_str("||"),
            '\'' => out.push_str("|'"),
            '\n' => out.push_str("|n"),
            '\r' => out.push_str("|r"),
            '[' => out.push_str("|["),
            ']' => out.push_str("|]"),
            other if other.is_control() => out.push(' '),
            other => out.push(other),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(name: &str, passed: bool, allow_fail: bool, diffs: Vec<Diff>) -> Outcome {
        Outcome {
            name: name.to_string(),
            weight: 1,
            allow_fail,
            passed,
            diffs,
            unexpected_calls: Vec::new(),
            unmentioned_files: Vec::new(),
            duration_ms: 12,
        }
    }

    fn diff(path: &str, expected: &str, got: &str) -> Diff {
        Diff {
            path: path.to_string(),
            expected: expected.to_string(),
            got: got.to_string(),
        }
    }

    fn rendered(outcomes: Vec<Outcome>) -> String {
        let mut buffer = Vec::new();
        {
            let mut sink = TeamCity::new(&mut buffer);
            for outcome in &outcomes {
                sink.case_finished(outcome);
            }
            sink.finish(&Report::from(outcomes));
        }
        String::from_utf8_lossy(&buffer).into_owned()
    }

    /// The suite opens once and closes once, whatever happened in between.
    #[test]
    fn the_suite_is_opened_and_closed_exactly_once() {
        let text = rendered(vec![
            outcome("one", true, false, Vec::new()),
            outcome("two", true, false, Vec::new()),
        ]);

        assert_eq!(text.matches("testSuiteStarted").count(), 1);
        assert_eq!(text.matches("testSuiteFinished").count(), 1);
        assert!(
            text.find("testSuiteStarted") < text.find("testStarted name='one'"),
            "and the suite opens before the first case: {text}"
        );
    }

    /// An empty run still produces a matched pair.
    ///
    /// One unmatched close is a protocol error to the reader, and an IDE shows it as such — which is a
    /// worse answer than an empty tree to a suite that selected no case.
    #[test]
    fn an_empty_run_still_opens_the_suite_it_closes() {
        let text = rendered(Vec::new());

        assert_eq!(text.matches("testSuiteStarted").count(), 1);
        assert_eq!(text.matches("testSuiteFinished").count(), 1);
    }

    /// A failure is reported as a comparison, which is what opens the IDE's diff viewer.
    ///
    /// **The reason this renderer is worth having.** A `Diff` is already a path, a wanted value and a
    /// found one; every other format has to flatten those three into prose, and this one does not.
    #[test]
    fn a_failure_carries_the_two_values_as_a_comparison() {
        let text = rendered(vec![outcome(
            "banner",
            false,
            false,
            vec![diff(
                "expect.stdout.equals",
                "my-tool 1.2.4",
                "my-tool 1.2.3",
            )],
        )]);

        assert!(text.contains("type='comparisonFailure'"), "{text}");
        assert!(text.contains("expected='my-tool 1.2.4'"), "{text}");
        assert!(text.contains("actual='my-tool 1.2.3'"), "{text}");
        assert!(
            text.contains("message='expect.stdout.equals'"),
            "and the collapsed node names the assertion: {text}"
        );
    }

    /// A failure with nothing to compare is still a failure.
    #[test]
    fn a_failure_with_no_diffs_is_reported_without_a_comparison() {
        let text = rendered(vec![outcome("stray", false, false, Vec::new())]);

        assert!(text.contains("testFailed name='stray'"), "{text}");
        assert!(
            !text.contains("comparisonFailure"),
            "an unexpected call compares nothing: {text}"
        );
        assert!(text.contains("catch-all"), "{text}");
    }

    /// A tolerated failure is ignored, not failed — the same mapping JUnit makes.
    #[test]
    fn a_tolerated_failure_is_ignored_rather_than_failed() {
        let text = rendered(vec![outcome(
            "known-gap",
            false,
            true,
            vec![diff("expect.exit_code", "0", "1")],
        )]);

        assert!(text.contains("testIgnored name='known-gap'"), "{text}");
        assert!(
            !text.contains("testFailed"),
            "failing would break the build the tolerance exists to keep green: {text}"
        );
    }

    /// Every case reports how long it took, in the unit the message wants.
    #[test]
    fn a_case_reports_its_duration() {
        let text = rendered(vec![outcome("timed", true, false, Vec::new())]);

        assert!(
            text.contains("testFinished name='timed' duration='12'"),
            "{text}"
        );
    }

    /// The escape character is replaced first, or it escapes its own escapes.
    ///
    /// Doing the bar after the others turns every newline into a literal `||n`, which an IDE renders as
    /// the four characters rather than as a line break — a whole failure body arriving as one line.
    #[test]
    fn the_escape_character_is_handled_before_the_ones_it_introduces() {
        assert_eq!(escaped("a\nb"), "a|nb");
        assert_eq!(escaped("a|b"), "a||b");
        assert_eq!(escaped("it's"), "it|'s");
        assert_eq!(escaped("[x]"), "|[x|]");
        assert_eq!(escaped("a\r\nb"), "a|r|nb");
        assert_eq!(
            escaped("a|\nb"),
            "a|||nb",
            "the bar becomes two, and the newline's own bar is not doubled after it"
        );
    }

    /// A control byte becomes a space rather than an escape.
    #[test]
    fn a_control_byte_does_not_reach_the_message() {
        let escaped = escaped("bell\u{7}here");

        assert_eq!(escaped, "bell here");
        assert!(
            !escaped.contains('\u{7}'),
            "a subject that printed a bell is not saying something a test tree should reproduce"
        );
    }

    /// A subject's own output cannot forge a message.
    ///
    /// The one thing this format is exposed to: an assertion value containing `##teamcity[…]` would be
    /// read as a second message if the brackets survived. They do not.
    #[test]
    fn a_value_that_looks_like_a_message_cannot_forge_one() {
        let text = rendered(vec![outcome(
            "sneaky",
            false,
            false,
            vec![diff(
                "expect.stdout.equals",
                "clean",
                "##teamcity[testFinished name='sneaky']",
            )],
        )]);

        // Counted as *lines that are messages*, not as occurrences in the text: the legitimate messages
        // contain the same prefix, so counting occurrences measured the renderer's own output. Five is
        // the whole conversation — suite open, case start, failure, case finish, suite close.
        let messages = text
            .lines()
            .filter(|line| line.starts_with("##teamcity["))
            .count();

        assert_eq!(
            messages, 5,
            "a value must not be able to add a message of its own: {text}"
        );
        assert!(
            text.contains("##teamcity|[testFinished"),
            "the forged text is carried, escaped, so the reader still sees what the subject wrote: \
             {text}"
        );
    }
}
