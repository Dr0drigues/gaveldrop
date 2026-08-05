//! Expectations on the call journal.

use std::collections::BTreeMap;

use gaveldrop_fake::Call;

use crate::verdict::Diff;

/// Checks declared per-binary call counts against the journal, rooting every diff at `at`.
///
/// A declared count of `0` is not a formality: asserting that a dependency was **not**
/// touched is often the interesting half of a case, because the answer can be right while
/// the side effect was wrong.
///
/// **`at` rather than a hard-coded `expect`.** This was the one check of the seven that wrote its own
/// root, so a count violated inside an exchange was reported as `expect.calls.git` — sending the
/// reader to the case's own `expect:` block, which was correct. Its six neighbours all take the
/// prefix; found by the consumer who read the six and then the seventh.
pub fn check(expected: &BTreeMap<String, usize>, calls: &[Call], at: &str) -> Vec<Diff> {
    let mut diffs = Vec::new();

    for (bin, want) in expected {
        let got = calls.iter().filter(|call| &call.bin == bin).count();
        if got != *want {
            diffs.push(Diff {
                path: format!("{at}.calls.{bin}"),
                expected: want.to_string(),
                got: got.to_string(),
                help: None,
            });
        }
    }

    diffs
}

/// The binaries that reached the catch-all, in first-seen order.
///
/// These fail a case whether or not it mentions calls: an unexpected call is exactly what
/// the catch-all exists to make loud.
pub fn unexpected(calls: &[Call]) -> Vec<String> {
    let mut seen = Vec::new();

    for call in calls.iter().filter(|call| call.catch_all) {
        if !seen.contains(&call.bin) {
            seen.push(call.bin.clone());
        }
    }

    seen
}
