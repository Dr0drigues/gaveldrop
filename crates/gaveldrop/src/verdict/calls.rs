//! Expectations on the call journal.

use std::collections::BTreeMap;

use gaveldrop_fake::Call;

use crate::verdict::Diff;

/// Checks declared per-binary call counts against the journal.
///
/// A declared count of `0` is not a formality: asserting that a dependency was **not**
/// touched is often the interesting half of a case, because the answer can be right while
/// the side effect was wrong.
pub fn check(expected: &BTreeMap<String, usize>, calls: &[Call]) -> Vec<Diff> {
    let mut diffs = Vec::new();

    for (bin, want) in expected {
        let got = calls.iter().filter(|call| &call.bin == bin).count();
        if got != *want {
            diffs.push(Diff {
                path: format!("expect.calls.{bin}"),
                expected: want.to_string(),
                got: got.to_string(),
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
