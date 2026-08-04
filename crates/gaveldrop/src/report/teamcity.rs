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

use std::collections::BTreeMap;
use std::io::Write;

use crate::report::{Report, Sink, failure_lines};
use crate::{Diff, Observations, Outcome};

/// Writes service messages as each case finishes.
pub struct TeamCity<W: Write> {
    out: W,
    opened: bool,
    seen: BTreeMap<String, Observations>,
    declared: BTreeMap<String, Vec<Option<String>>>,
}

/// The suite name the tree is rooted at.
const SUITE: &str = "gaveldrop";

impl<W: Write> TeamCity<W> {
    /// A renderer writing into `out`, which has to be the standard output an IDE is reading.
    pub fn new(out: W) -> Self {
        Self {
            out,
            opened: false,
            seen: BTreeMap::new(),
            declared: BTreeMap::new(),
        }
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
    /// Kept so the case's own node can show what the subject did.
    ///
    /// **A node with nothing under it is half a test tree.** The two messages that open and close a case
    /// are both written *after* it ran, so there is no window a reader's output could land in — clicking a
    /// case showed the suite's summary instead of that case's run. The observations are what the HTML
    /// report folds open, and they are what belongs here too.
    fn observed(&mut self, case: &str, observations: &Observations) {
        self.seen.insert(case.to_string(), observations.clone());
    }

    fn declares_steps(&mut self, case: &str, names: &[Option<String>]) {
        self.declared.insert(case.to_string(), names.to_vec());
    }

    fn case_finished(&mut self, outcome: &Outcome) {
        self.open();

        if let Some(steps) = self.declared.get(&outcome.name).cloned() {
            self.nested(outcome, &steps);
            return;
        }

        let name = escaped(&outcome.name);

        let _ = writeln!(self.out, "##teamcity[testStarted name='{name}']");

        for line in detail(outcome, self.seen.get(&outcome.name), false) {
            let _ = writeln!(
                self.out,
                "##teamcity[testStdOut name='{name}' out='{}']",
                escaped(&line)
            );
        }

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

impl<W: Write> TeamCity<W> {
    /// A case that declared exchanges, as a suite with one node per exchange.
    ///
    /// **The shape follows the case rather than the report.** A case with `steps:` is several exchanges
    /// with their own expectations, and flattening them meant a failure said `steps[2].status` in prose
    /// where a tree can say *which* exchange, at a glance.
    ///
    /// The run as a whole gets a node of its own, because `expect:` at the top level describes what the
    /// case produced *across* the exchanges — the files it wrote, the events it emitted, the invariants
    /// that must hold — and those assertions would otherwise have nowhere to be reported.
    ///
    /// **Only the whole-run node carries a duration.** A case is timed; an exchange is not, and a
    /// fabricated `duration='0'` on every step would read as an exchange that took no time rather than as
    /// one nobody measured.
    fn nested(&mut self, outcome: &Outcome, steps: &[Option<String>]) {
        let suite = escaped(&outcome.name);
        let _ = writeln!(self.out, "##teamcity[testSuiteStarted name='{suite}']");

        let whole = escaped(WHOLE_RUN);
        let _ = writeln!(self.out, "##teamcity[testStarted name='{whole}']");
        for line in detail(outcome, self.seen.get(&outcome.name), false) {
            let _ = writeln!(
                self.out,
                "##teamcity[testStdOut name='{whole}' out='{}']",
                escaped(&line)
            );
        }
        self.verdict(&whole, outcome, &sifted(outcome, None));
        let _ = writeln!(
            self.out,
            "##teamcity[testFinished name='{whole}' duration='{}']",
            outcome.duration_ms
        );

        for (index, declared) in steps.iter().enumerate() {
            let label = escaped(&match declared {
                Some(given) => given.clone(),
                None => format!("step {}", index + 1),
            });

            let _ = writeln!(self.out, "##teamcity[testStarted name='{label}']");
            let seen = self
                .seen
                .get(&outcome.name)
                .and_then(|observations| observations.steps.get(index));
            for line in detail(outcome, seen, true) {
                let _ = writeln!(
                    self.out,
                    "##teamcity[testStdOut name='{label}' out='{}']",
                    escaped(&line)
                );
            }
            self.verdict(&label, outcome, &sifted(outcome, Some(index)));
            let _ = writeln!(self.out, "##teamcity[testFinished name='{label}']");
        }

        let _ = writeln!(self.out, "##teamcity[testSuiteFinished name='{suite}']");
        let _ = self.out.flush();
    }

    /// One node's verdict, from the diffs that belong to it.
    fn verdict(&mut self, name: &str, outcome: &Outcome, mine: &[Diff]) {
        if mine.is_empty() {
            return;
        }

        let detail = escaped(&mine.iter().map(described).collect::<Vec<_>>().join("\n"));

        if outcome.allow_fail {
            let _ = writeln!(
                self.out,
                "##teamcity[testIgnored name='{name}' message='tolerated: {}']",
                escaped(&summarise(mine))
            );
        } else {
            let first = &mine[0];
            let _ = writeln!(
                self.out,
                "##teamcity[testFailed type='comparisonFailure' name='{name}' message='{}' \
                 details='{detail}' expected='{}' actual='{}']",
                escaped(&first.path),
                escaped(&first.expected),
                escaped(&first.got)
            );
        }
    }
}

/// The name of the node carrying what the case asserted across its exchanges.
const WHOLE_RUN: &str = "the run as a whole";

/// The diffs belonging to one exchange, or to everything that is not one.
///
/// By the path, which is where the verdict already records this: a step's assertions are rooted at
/// `steps[<index>]`, optionally followed by the name the step gave itself.
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

/// One diff as the line a reader reads.
fn described(diff: &Diff) -> String {
    format!(
        "    {}\n      expected  {}\n      got       {}",
        diff.path, diff.expected, diff.got
    )
}

/// What the subject did, for the case's own node.
///
/// The same content the HTML report folds open, because the question is the same: a case can pass and
/// still have done something you did not expect, and a node saying only `ok` cannot show it. Read from the
/// observations rather than reconstructed from the verdict — the verdict says what was *asserted*, and this
/// is for everything else.
///
/// Streams are cut with the full length named. A subject that printed a megabyte would otherwise put a
/// megabyte under one node, and a reader looking for the first line would not find it.
fn detail(outcome: &Outcome, observed: Option<&Observations>, exchange: bool) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some(observations) = observed {
        // A zero exit on an exchange is either "it went fine" or "nobody measured one" — the web adapter
        // records a status per exchange and leaves this at its default — and a line that cannot tell the
        // two apart is a fabricated measurement. On a whole run it is always measured, so it always shows.
        if !exchange || observations.exit != 0 {
            lines.push(format!("exit {}", observations.exit));
        }

        if let Some(status) = observations.status {
            lines.push(format!("status {status}"));
        }
        if !observations.stdout.is_empty() {
            lines.push(format!("stdout: {}", capped(&observations.stdout)));
        }
        if !observations.stderr.is_empty() {
            lines.push(format!("stderr: {}", capped(&observations.stderr)));
        }
        if !observations.calls.is_empty() {
            lines.push(format!("calls: {}", tallied(observations)));
        }
        if !observations.files.is_empty() {
            let written: Vec<String> = observations
                .files
                .iter()
                .map(|effect| format!("{} ({} bytes)", effect.path.display(), effect.size))
                .collect();
            lines.push(format!("files: {}", written.join(", ")));
        }
    }

    // Offered here as everywhere else: it is often where you find what you should have been asserting.
    if !outcome.unmentioned_files.is_empty() {
        lines.push(format!(
            "also written, not asserted: {}",
            outcome.unmentioned_files.join(", ")
        ));
    }

    lines
}

/// One binary per line with how often it was called, and whether the catch-all answered.
fn tallied(observations: &Observations) -> String {
    let mut counted: BTreeMap<&str, (usize, bool)> = BTreeMap::new();

    for call in &observations.calls {
        let entry = counted.entry(call.bin.as_str()).or_insert((0, false));
        entry.0 += 1;
        entry.1 |= call.catch_all;
    }

    counted
        .into_iter()
        .map(|(bin, (times, caught))| {
            let unexpected = if caught {
                ", reached the catch-all"
            } else {
                ""
            };
            format!("{bin} ×{times}{unexpected}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// As much of a stream as helps, naming what was left out.
fn capped(stream: &str) -> String {
    const ROOM: usize = 2_000;

    let trimmed = stream.trim_end();
    match trimmed.char_indices().nth(ROOM) {
        Some((cut, _)) => format!("{}… ({} bytes in all)", &trimmed[..cut], stream.len()),
        None => trimmed.to_string(),
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

    /// A passing case's node carries what the subject did, not just the word `ok`.
    ///
    /// **The gap that made the tree half a tree.** `testStarted` and `testFinished` are both written after
    /// the case ran, so there is no window a reader's output could land in — clicking a case showed the
    /// suite's summary instead of that case's run. This is the same content the HTML report folds open, and
    /// it answers the question a verdict does not: a case can pass and still have done something you did
    /// not expect.
    #[test]
    fn a_case_node_shows_what_the_subject_did() {
        let mut buffer = Vec::new();
        let passing = outcome("fine", true, false, Vec::new());
        {
            let mut sink = TeamCity::new(&mut buffer);
            sink.observed(
                "fine",
                &Observations {
                    exit: 0,
                    stdout: "the banner".to_string(),
                    stderr: "a warning".to_string(),
                    ..Default::default()
                },
            );
            sink.case_finished(&passing);
            sink.finish(&Report::from(vec![passing]));
        }
        let text = String::from_utf8_lossy(&buffer).into_owned();

        assert!(
            text.contains("testStdOut name='fine' out='exit 0'"),
            "{text}"
        );
        assert!(text.contains("stdout: the banner"), "{text}");
        assert!(text.contains("stderr: a warning"), "{text}");
        assert!(
            text.find("testStarted name='fine'") < text.find("testStdOut name='fine'"),
            "and it arrives inside the case, or it belongs to no node: {text}"
        );
    }

    /// A case nobody observed still reports, with nothing invented.
    ///
    /// A setup failure never reaches an adapter, so no observations exist for it — and a node claiming
    /// `exit 0` there would be a fabrication.
    #[test]
    fn a_case_with_no_observations_reports_no_run() {
        let text = rendered(vec![outcome(
            "never-ran",
            false,
            false,
            vec![diff("setup", "it runs", "no adapter")],
        )]);

        assert!(
            !text.contains("testStdOut"),
            "nothing was observed, so nothing is claimed: {text}"
        );
        assert!(text.contains("testFailed"), "{text}");
    }

    /// Files the case said nothing about are offered on the node, as everywhere else.
    #[test]
    fn files_nobody_asserted_are_offered_on_the_node() {
        let mut written = outcome("wrote-more", true, false, Vec::new());
        written.unmentioned_files = vec!["orders.log".to_string()];

        let mut buffer = Vec::new();
        {
            let mut sink = TeamCity::new(&mut buffer);
            sink.observed("wrote-more", &Observations::default());
            sink.case_finished(&written);
        }

        assert!(
            String::from_utf8_lossy(&buffer).contains("also written, not asserted: orders.log"),
            "it is often where you find what you should have been asserting"
        );
    }

    /// A long stream is cut, with its full length named.
    #[test]
    fn a_long_stream_does_not_fill_one_node() {
        let mut buffer = Vec::new();
        let passing = outcome("chatty", true, false, Vec::new());
        {
            let mut sink = TeamCity::new(&mut buffer);
            sink.observed(
                "chatty",
                &Observations {
                    stdout: "x".repeat(50_000),
                    ..Default::default()
                },
            );
            sink.case_finished(&passing);
        }
        let text = String::from_utf8_lossy(&buffer).into_owned();

        assert!(text.contains("50000 bytes in all"), "the length is named");
        assert!(
            text.len() < 6_000,
            "and a reader looking for the first line can still find it: {} bytes",
            text.len()
        );
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

    /// A case that declared exchanges becomes a suite of them.
    ///
    /// **The shape follows the case rather than the report.** Flattening meant a failure said
    /// `steps[2].status` in prose where a tree can name which exchange, at a glance.
    #[test]
    fn a_case_with_exchanges_becomes_a_suite_of_them() {
        let mut buffer = Vec::new();
        let passing = outcome("ordering", true, false, Vec::new());
        {
            let mut sink = TeamCity::new(&mut buffer);
            sink.declares_steps("ordering", &[Some("creates an order".to_string()), None]);
            sink.case_finished(&passing);
        }
        let text = String::from_utf8_lossy(&buffer).into_owned();

        assert!(text.contains("testSuiteStarted name='ordering'"), "{text}");
        assert!(
            text.contains("testStarted name='the run as a whole'"),
            "`expect:` at the top level describes what the case produced across the exchanges, so it \
             needs a node of its own or those assertions have nowhere to be reported: {text}"
        );
        assert!(
            text.contains("testStarted name='creates an order'"),
            "a step's own name, which is why the trait carries what the case declared: {text}"
        );
        assert!(
            text.contains("testStarted name='step 2'"),
            "and an index where the case gave no name: {text}"
        );
        assert!(text.contains("testSuiteFinished name='ordering'"), "{text}");
    }

    /// Only the whole run is timed, because only the whole run is measured.
    #[test]
    fn an_exchange_reports_no_duration_it_does_not_have() {
        let mut buffer = Vec::new();
        let passing = outcome("ordering", true, false, Vec::new());
        {
            let mut sink = TeamCity::new(&mut buffer);
            sink.declares_steps("ordering", &[None]);
            sink.case_finished(&passing);
        }
        let text = String::from_utf8_lossy(&buffer).into_owned();

        assert!(
            text.contains("testFinished name='the run as a whole' duration='12'"),
            "{text}"
        );
        assert!(
            text.contains("testFinished name='step 1']"),
            "a case is timed and an exchange is not; `duration='0'` would read as an exchange that took \
             no time rather than as one nobody measured: {text}"
        );
    }

    /// Each failure lands on the node it belongs to, by the path the verdict already gave it.
    #[test]
    fn a_failure_lands_on_the_exchange_it_came_from() {
        let mut buffer = Vec::new();
        let failing = outcome(
            "ordering",
            false,
            false,
            vec![
                diff("expect.exit_code", "3", "0"),
                diff("steps[1] \"the second\".status", "201", "500"),
            ],
        );
        {
            let mut sink = TeamCity::new(&mut buffer);
            sink.declares_steps("ordering", &[None, Some("the second".to_string())]);
            sink.case_finished(&failing);
        }
        let text = String::from_utf8_lossy(&buffer).into_owned();

        assert!(
            text.contains("testFailed type='comparisonFailure' name='the run as a whole' message='expect.exit_code'"),
            "a whole-run assertion is not an exchange's fault: {text}"
        );
        assert!(
            text.contains("name='the second' message='steps|[1|] \"the second\".status'"),
            "and an exchange's is its own: {text}"
        );
        assert!(
            !text.contains("name='step 1' message="),
            "the exchange that held is left alone: {text}"
        );
    }

    /// A case that declared nothing stays one node, with no suite wrapped around it.
    #[test]
    fn a_case_with_no_exchanges_is_not_wrapped_in_a_suite() {
        let text = rendered(vec![outcome("plain", true, false, Vec::new())]);

        assert_eq!(
            text.matches("testSuiteStarted").count(),
            1,
            "only the run's own suite; nesting a single case inside one of its own would be a level \
             that says nothing: {text}"
        );
        assert!(text.contains("testStarted name='plain'"), "{text}");
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
