//! Expectations on a stream of text.

use crate::TextExpectation;
use crate::verdict::Diff;

/// Checks `expectation` against `stream`, prefixing every diff path with `prefix`.
///
/// `prefix` is what makes a failure locatable — `expect.stdout.contains[1]` rather than
/// "a contains expectation failed".
pub fn check(expectation: &TextExpectation, stream: &str, prefix: &str) -> Vec<Diff> {
    let mut diffs = Vec::new();

    for (index, needle) in expectation.contains.iter().enumerate() {
        if !stream.contains(needle.as_str()) {
            diffs.push(Diff {
                path: format!("{prefix}.contains[{index}]"),
                expected: format!("contains {needle:?}"),
                got: excerpt(stream),
            });
        }
    }

    for (index, needle) in expectation.absent.iter().enumerate() {
        if let Some(at) = stream.find(needle.as_str()) {
            diffs.push(Diff {
                path: format!("{prefix}.absent[{index}]"),
                expected: format!("nowhere: {needle:?}"),
                got: around(stream, at),
            });
        }
    }

    diffs
}

/// What the stream held, for a failure message: as much as fits, on one line, readable.
///
/// It used to be the **first line** of the stream. That reads as the whole answer and is not, and it
/// cost the first real consumer of the shell adapter most of a debugging session: their subject
/// started with a colour escape followed by a newline, so `got` was one invisible sequence and the
/// report showed an empty value. An empty `got` on a stream assertion means "the subject wrote
/// nothing", so they went looking for a function that was not running — and it was running fine.
///
/// Two things follow. The whole stream is shown rather than its first line, with newlines made
/// visible so it still occupies one line of the report; and control bytes are escaped, so a stream
/// that is invisible can never be mistaken for a stream that is absent.
fn excerpt(stream: &str) -> String {
    if stream.is_empty() {
        return "(empty)".to_string();
    }

    let shown = visible(stream.trim());
    if shown.is_empty() {
        return format!("({} bytes, all of them whitespace)", stream.len());
    }

    // 120, the same cap as before this changed. Widening it was tempting and pointless: the stream
    // that caused the trouble renders to about forty-five characters, so the cap was never what hid
    // it — taking only the first line was.
    capped(&shown, 120, stream.len())
}

/// The line containing byte offset `at`, so an `absent` failure shows the offender in
/// context rather than the whole stream.
fn around(stream: &str, at: usize) -> String {
    let start = stream[..at].rfind('\n').map_or(0, |index| index + 1);
    let end = stream[at..]
        .find('\n')
        .map_or(stream.len(), |index| at + index);
    let line = stream[start..end].trim();
    capped(&visible(line), 120, line.len())
}

/// Control bytes rendered so they can be seen.
///
/// A report is read by a person in a terminal, which *interprets* an escape sequence rather than
/// showing it — so the bytes that most need to be visible in a diagnostic are exactly the ones a
/// terminal hides. Escapes are the common case in shell output: any tool that colours its output
/// produces them, and `contains:` fails on them.
fn visible(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\n' => "⏎ ".to_string(),
            '\t' => "→ ".to_string(),
            '\u{1b}' => "\\e".to_string(),
            other if other.is_control() => format!("\\x{:02x}", other as u32),
            other => other.to_string(),
        })
        .collect()
}

/// Caps a fragment so one long stream cannot drown a report, naming what was left out.
///
/// The original length is the stream's, not the rendered one's: a reader wants to know how much the
/// subject wrote, and `visible` makes that number bigger for reasons of its own.
fn capped(text: &str, limit: usize, original: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let kept: String = text.chars().take(limit).collect();
    format!("{kept}… ({original} bytes in all)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextExpectation;

    fn contains(needle: &str) -> TextExpectation {
        TextExpectation {
            contains: vec![needle.to_string()],
            ..Default::default()
        }
    }

    fn absent(needle: &str) -> TextExpectation {
        TextExpectation {
            absent: vec![needle.to_string()],
            ..Default::default()
        }
    }

    #[test]
    fn a_stream_that_is_only_invisible_bytes_does_not_read_as_an_empty_one() {
        // What `_ui_header` produces: a colour escape, a newline, then the content. The old
        // excerpt took the first line, which was the escape alone, and rendered as nothing.
        let stream = "\u{1b}[1;32m\n== lazygit ==\nconfig: /nowhere\n";

        let diffs = check(
            &contains("un-chemin-qui-nexiste-pas"),
            stream,
            "expect.stdout",
        );

        assert_eq!(diffs.len(), 1);
        assert!(
            !diffs[0].got.trim().is_empty(),
            "an empty `got` on a stream assertion says the subject wrote nothing, and this subject \
             wrote three lines. That reading cost a consumer most of a session on a case that had \
             no problem: {:?}",
            diffs[0].got
        );
        assert!(
            diffs[0].got.contains("lazygit"),
            "and what it wrote has to be in there, not just the first line: {:?}",
            diffs[0].got
        );
        assert!(
            diffs[0].got.contains("\\e"),
            "the escape is shown rather than interpreted, because a terminal hides exactly the \
             bytes a diagnostic most needs to show — and `contains:` failed on them: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn newlines_are_visible_so_the_report_keeps_one_line_per_failure() {
        let diffs = check(&contains("nope"), "first\nsecond", "expect.stdout");

        assert!(
            !diffs[0].got.contains('\n'),
            "the terminal report aligns `got` in a column; a real newline would break the layout \
             of every failure after it: {:?}",
            diffs[0].got
        );
        assert!(
            diffs[0].got.contains("first") && diffs[0].got.contains("second"),
            "both lines are still there: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn a_truly_empty_stream_says_so() {
        let diffs = check(&contains("anything"), "", "expect.stdout");

        assert_eq!(
            diffs[0].got, "(empty)",
            "the honest case has to stay honest, or the fix above would have traded one \
             misleading message for another"
        );
    }

    #[test]
    fn whitespace_only_is_distinguished_from_empty() {
        let diffs = check(&contains("anything"), "  \n\t\n ", "expect.stdout");

        assert!(
            diffs[0].got.contains("whitespace"),
            "a subject that wrote six bytes of blanks did write something, and a reader chasing a \
             missing newline needs to know: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn a_long_stream_is_capped_and_names_what_it_left_out() {
        let stream = "x".repeat(5_000);

        let diffs = check(&contains("nope"), &stream, "expect.stdout");

        assert!(
            diffs[0].got.len() < 400,
            "one long stream must not drown the failures under it: {}",
            diffs[0].got.len()
        );
        assert!(
            diffs[0].got.contains("5000 bytes in all"),
            "and the reader is told how much the subject really wrote, so a cap is never mistaken \
             for the whole output: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn an_absent_failure_still_shows_the_offending_line_rather_than_the_stream() {
        let stream = "line one\nhere is the ZSH_ENV_DIR problem\nline three";

        let diffs = check(&absent("ZSH_ENV_DIR"), stream, "expect.stdout");

        assert_eq!(diffs.len(), 1);
        assert!(
            diffs[0].got.contains("ZSH_ENV_DIR problem"),
            "the point of an `absent` failure is where the needle is: {:?}",
            diffs[0].got
        );
        assert!(
            !diffs[0].got.contains("line three"),
            "and showing the whole stream would bury it: {:?}",
            diffs[0].got
        );
    }
}
