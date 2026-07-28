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

/// A short, single-line excerpt of a stream, for a failure message.
fn excerpt(stream: &str) -> String {
    let trimmed = stream.trim();
    if trimmed.is_empty() {
        return "(empty)".to_string();
    }
    truncate(trimmed.lines().next().unwrap_or(trimmed))
}

/// The line containing byte offset `at`, so an `absent` failure shows the offender in
/// context rather than the whole stream.
fn around(stream: &str, at: usize) -> String {
    let start = stream[..at].rfind('\n').map_or(0, |index| index + 1);
    let end = stream[at..]
        .find('\n')
        .map_or(stream.len(), |index| at + index);
    truncate(stream[start..end].trim())
}

/// Caps a fragment so one long line cannot drown a report.
fn truncate(text: &str) -> String {
    const LIMIT: usize = 120;

    if text.chars().count() <= LIMIT {
        return text.to_string();
    }
    let kept: String = text.chars().take(LIMIT).collect();
    format!("{kept}…")
}
