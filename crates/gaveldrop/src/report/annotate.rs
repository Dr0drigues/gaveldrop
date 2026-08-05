//! Failures as workflow commands, annotated on the case's own line.

use std::io::Write;
use std::path::PathBuf;

use crate::Outcome;
use crate::report::sources::Sources;
use crate::report::{Report, Sink, failure_lines};

/// Emits one annotation per failing case, on the line its assertion came from.
///
/// The documents come from [`Sources`], which reads them rather than the trait growing a way to carry
/// them — and which the terminal renderer now shares, so an annotation and a printed `--> path:line`
/// cannot disagree about where a failure lives.
pub struct Annotate<W: Write> {
    out: W,
    sources: Sources,
}

impl<W: Write> Annotate<W> {
    /// A renderer that can locate any case among `paths`.
    pub fn new(out: W, paths: &[PathBuf]) -> Self {
        Self {
            out,
            sources: Sources::load(paths),
        }
    }
}

impl<W: Write> Sink for Annotate<W> {
    fn case_finished(&mut self, outcome: &Outcome) {
        if outcome.passed {
            return;
        }

        let level = if outcome.allow_fail {
            "warning"
        } else {
            "error"
        };
        let Some(found) = self.sources.locate(&outcome.name, first_path(outcome)) else {
            let _ = writeln!(
                self.out,
                "::{level}::{}",
                encoded(&message(outcome, &outcome.name))
            );
            return;
        };

        let _ = writeln!(
            self.out,
            "::{level} file={},line={},title={}::{}",
            property(&found.path.to_string_lossy()),
            found.line,
            property(&outcome.name),
            encoded(&message(outcome, &outcome.name))
        );
        let _ = self.out.flush();
    }

    fn finish(&mut self, _report: &Report) {
        let _ = self.out.flush();
    }
}

/// The assertion path an annotation points at.
///
/// The first one. A case with four broken assertions gets one annotation on the first, not four on
/// four lines: a pull request buried in annotations is a pull request nobody reads, and the full list
/// is in the message.
fn first_path(outcome: &Outcome) -> &str {
    outcome
        .diffs
        .first()
        .map(|diff| diff.path.as_str())
        .unwrap_or("expect")
}

/// The whole failure, as one message.
fn message(outcome: &Outcome, name: &str) -> String {
    let mut text = format!("{name}\n");
    for line in failure_lines(outcome) {
        text.push_str(line.trim_end());
        text.push('\n');
    }
    text
}

/// A workflow command's message, with what would end the command encoded.
///
/// A command is **one line**. A subject whose output contained a newline would otherwise truncate the
/// annotation and leave the rest as junk in the log — which is why this is not optional. The three
/// sequences are GitHub's, and they are not guessable, so they are written down in
/// `CONTRIBUTING.md` too.
fn encoded(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// A value inside `file=` or `title=`, where a comma or a colon would end the field.
fn property(text: &str) -> String {
    encoded(text).replace(':', "%3A").replace(',', "%2C")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Diff;

    const DOCUMENT: &str = r#"name: an-order-is-created
weight: 8
setup:
  run: ["true"]
expect:
  exit_code: 0
  stdout:
    contains: ["created"]
"#;

    fn written(outcome: &Outcome, with_document: bool) -> String {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();

        if with_document {
            let path = dir.path().join("an-order.yaml");
            std::fs::write(&path, DOCUMENT).unwrap();
            paths.push(path);
        }

        let mut buffer = Vec::new();
        {
            let mut sink = Annotate::new(&mut buffer, &paths);
            sink.case_finished(outcome);
            sink.finish(&Report::from(vec![outcome.clone()]));
        }
        String::from_utf8_lossy(&buffer).into_owned()
    }

    fn outcome(passed: bool, allow_fail: bool, diffs: Vec<Diff>) -> Outcome {
        Outcome {
            name: "an-order-is-created".to_string(),
            weight: 8,
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
            expected: "0".to_string(),
            got: got.to_string(),
            help: None,
        }
    }

    #[test]
    fn a_failure_names_the_file_the_line_and_the_case() {
        let text = written(
            &outcome(
                false,
                false,
                vec![diff("expect.stdout.contains[0]", "nothing")],
            ),
            true,
        );

        assert!(text.starts_with("::error file="), "got {text}");
        assert!(
            text.contains("line=8"),
            "the assertion is on the `contains:` line, and an annotation on `expect:` would send \
             the reader to the right block and the wrong line: {text}"
        );
        assert!(text.contains("an-order-is-created"), "got {text}");
    }

    #[test]
    fn a_passing_case_produces_nothing() {
        assert!(
            written(&outcome(true, false, Vec::new()), true).is_empty(),
            "an annotation per passing case would bury the failures, which is the one thing this \
             output exists to surface"
        );
    }

    #[test]
    fn a_tolerated_failure_is_a_warning_rather_than_an_error() {
        let text = written(
            &outcome(false, true, vec![diff("expect.exit_code", "7")]),
            true,
        );

        assert!(
            text.starts_with("::warning"),
            "it must be visible without failing the check, which is exactly what `allow_fail` \
             asks for: {text}"
        );
    }

    #[test]
    fn a_newline_in_a_message_is_encoded_rather_than_ending_the_command() {
        let text = written(
            &outcome(
                false,
                false,
                vec![diff("expect.stdout", "line one\nline two")],
            ),
            true,
        );

        assert_eq!(
            text.lines().count(),
            1,
            "a workflow command is one line. A subject whose output contained a newline would \
             truncate the annotation and leave the rest as junk in the log: {text}"
        );
        assert!(text.contains("%0A"), "got {text}");
    }

    #[test]
    fn a_percent_is_encoded_before_anything_else() {
        let text = written(
            &outcome(
                false,
                false,
                vec![diff("expect.stdout", "100%0Anot a newline")],
            ),
            true,
        );

        assert!(
            text.contains("100%250A"),
            "encoding the newline first would turn a literal `%0A` in the subject's output into a \
             real line break on the way out: {text}"
        );
    }

    #[test]
    fn a_case_whose_document_is_missing_is_still_annotated_without_a_line() {
        let text = written(
            &outcome(false, false, vec![diff("expect.exit_code", "1")]),
            false,
        );

        assert!(
            text.starts_with("::error::"),
            "a failure with no document to point at is still a failure worth surfacing: {text}"
        );
    }

    #[test]
    fn several_broken_assertions_produce_one_annotation() {
        let text = written(
            &outcome(
                false,
                false,
                vec![
                    diff("expect.exit_code", "1"),
                    diff("expect.stdout.contains[0]", "nothing"),
                ],
            ),
            true,
        );

        assert_eq!(
            text.lines().count(),
            1,
            "a pull request buried in annotations is one nobody reads, and the full list is in the \
             message: {text}"
        );
        assert!(text.contains("expect.stdout"), "got {text}");
    }
}
