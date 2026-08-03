//! Comparing response headers.
//!
//! Its own module for one reason: header names are case-insensitive, and that rule has to be applied
//! in exactly one place. A case asserting `Content-Type` against a server sending `content-type`
//! would otherwise be testing the server's spelling rather than its behaviour.

use std::collections::BTreeMap;

use crate::TextExpectation;
use crate::verdict::{Diff, text};

/// Checks each expected header against what the response carried.
///
/// A header the response never sent is reported as absent rather than compared against an empty
/// string: "no such header" and "the header said something else" are different failures, and a
/// reader fixing one needs to know which they have.
pub fn check(
    expected: &BTreeMap<String, TextExpectation>,
    received: &BTreeMap<String, String>,
    at: &str,
) -> Vec<Diff> {
    let folded: BTreeMap<String, &String> = received
        .iter()
        .map(|(name, value)| (name.to_lowercase(), value))
        .collect();

    expected
        .iter()
        .flat_map(
            |(name, expectation)| match folded.get(&name.to_lowercase()) {
                Some(value) => text::check(expectation, value, &format!("{at}.headers.{name}")),
                None => vec![Diff {
                    path: format!("{at}.headers.{name}"),
                    expected: "the header is present".to_string(),
                    got: if received.is_empty() {
                        "the response carried no headers".to_string()
                    } else {
                        format!("only {}", names_of(received))
                    },
                }],
            },
        )
        .collect()
}

/// The header names a response carried, for a failure that has to say what *was* there.
fn names_of(received: &BTreeMap<String, String>) -> String {
    received
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expecting(name: &str, contains: &str) -> BTreeMap<String, TextExpectation> {
        BTreeMap::from([(
            name.to_string(),
            TextExpectation {
                contains: vec![contains.to_string()],
                absent: Vec::new(),
                ..Default::default()
            },
        )])
    }

    fn received(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn a_name_matches_whatever_case_either_side_used() {
        assert!(
            check(
                &expecting("Content-Type", "json"),
                &received(&[("content-type", "application/json")]),
                "expect"
            )
            .is_empty(),
            "header names are case-insensitive, so a case must not depend on the server's spelling"
        );
    }

    #[test]
    fn a_missing_header_says_what_the_response_did_carry() {
        let diffs = check(
            &expecting("X-Request-Id", "-"),
            &received(&[("content-type", "text/plain")]),
            "expect",
        );

        assert_eq!(diffs[0].path, "expect.headers.X-Request-Id");
        assert!(
            diffs[0].got.contains("content-type"),
            "listing what was there turns a dead end into the next step — usually a typo in the \
             name: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn a_header_with_no_headers_at_all_says_so_plainly() {
        let diffs = check(&expecting("Content-Type", "json"), &received(&[]), "expect");

        assert!(
            diffs[0].got.contains("no headers"),
            "`only ` followed by nothing would read like a truncated message: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn a_present_header_with_the_wrong_value_reports_the_value() {
        let diffs = check(
            &expecting("Content-Type", "json"),
            &received(&[("Content-Type", "text/html")]),
            "expect",
        );

        assert!(
            diffs[0].got.contains("text/html"),
            "a wrong value must show the value, which is a different failure from an absent \
             header: {:?}",
            diffs[0].got
        );
    }
}
