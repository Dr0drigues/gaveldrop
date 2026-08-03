//! Asserting on a value inside a JSON body.
//!
//! Exists because GraphQL answers `200` for a failed operation and puts the failure in
//! `body.errors`, so a status is not enough. `body.contains` is not enough either: it matches a
//! substring of a serialisation, sensitive to spacing and key order, and `absent: ["errors"]` is
//! defeated by the word appearing in a product name.
//!
//! **The path syntax is deliberately poor** — dotted keys and numeric indices, nothing else. No
//! wildcards, no filters, no recursion. A query language would be the computation a case format must
//! not grow, and the hooks exist for anything that needs one.

use std::collections::BTreeMap;

use crate::TextExpectation;
use crate::verdict::{Diff, text};

/// The value at `path` in `body`, or `None` if either the body or the path does not lead there.
///
/// A segment indexes an object by key or an array by number. `None` covers a body that is not JSON,
/// a key that is absent, an index past the end, and a path that continues past a scalar — the caller
/// tells those apart, because they are different mistakes.
pub fn at(body: &str, path: &str) -> Option<serde_json::Value> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    let mut here = &parsed;

    for segment in path.split('.') {
        here = match here {
            serde_json::Value::Object(fields) => fields.get(segment)?,
            serde_json::Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }

    Some(here.clone())
}

/// Checks each expected path against the body.
pub fn check(expected: &BTreeMap<String, TextExpectation>, body: &str, at_path: &str) -> Vec<Diff> {
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(body);

    expected
        .iter()
        .flat_map(|(path, expectation)| {
            let where_it_is = format!("{at_path}.json.{path}");

            let Ok(root) = &parsed else {
                return vec![Diff {
                    path: where_it_is,
                    expected: format!("a JSON body with {path}"),
                    got: format!("a body that is not JSON: {}", truncated(body)),
                }];
            };

            match at(body, path) {
                Some(value) => text::check(expectation, &as_text(&value), &where_it_is),
                None => vec![Diff {
                    path: where_it_is,
                    expected: "the path leads somewhere".to_string(),
                    got: missing(root, path),
                }],
            }
        })
        .collect()
}

/// A JSON value as the text an expectation compares against.
///
/// A string loses its quotes, everything else is rendered as JSON. So a case writes `CHR-1` rather
/// than `"CHR-1"`, and `7` matches the number 7 without minding which JSON type it arrived as.
fn as_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// What to say about a path that led nowhere.
///
/// Names the keys that *were* there at the deepest point the path did reach. The cause is almost
/// always a spelling mistake, and a list is what makes it visible without opening the response by
/// hand.
fn missing(root: &serde_json::Value, path: &str) -> String {
    let mut here = root;
    let mut reached = String::new();

    for segment in path.split('.') {
        let next = match here {
            serde_json::Value::Object(fields) => fields.get(segment),
            serde_json::Value::Array(items) => {
                segment.parse::<usize>().ok().and_then(|at| items.get(at))
            }
            _ => None,
        };

        match next {
            Some(value) => {
                if !reached.is_empty() {
                    reached.push('.');
                }
                reached.push_str(segment);
                here = value;
            }
            None => return stopped_at(here, &reached, segment),
        }
    }

    "nothing".to_string()
}

/// The failure sentence for one segment that did not resolve.
fn stopped_at(here: &serde_json::Value, reached: &str, segment: &str) -> String {
    let holds = match here {
        serde_json::Value::Object(fields) => {
            let names: Vec<&str> = fields.keys().map(String::as_str).collect();
            if names.is_empty() {
                "no keys".to_string()
            } else {
                format!("keys {}", names.join(", "))
            }
        }
        serde_json::Value::Array(items) => format!("{} items", items.len()),
        scalar => format!("the scalar {}", truncated(&scalar.to_string())),
    };

    if reached.is_empty() {
        format!("no `{segment}`: the body holds {holds}")
    } else {
        format!("no `{segment}` under `{reached}`, which holds {holds}")
    }
}

/// A body short enough to read in a failure line.
fn truncated(body: &str) -> String {
    const LIMIT: usize = 120;
    let trimmed = body.trim();

    if trimmed.chars().count() <= LIMIT {
        return format!("{trimmed:?}");
    }

    let head: String = trimmed.chars().take(LIMIT).collect();
    format!("{head:?} (truncated)")
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{"data":{"order":{"id":7,"items":[{"sku":"CHR-1"}]}},"errors":null}"#;

    fn expecting(path: &str, contains: &str) -> BTreeMap<String, TextExpectation> {
        BTreeMap::from([(
            path.to_string(),
            TextExpectation {
                contains: vec![contains.to_string()],
                absent: Vec::new(),
                ..Default::default()
            },
        )])
    }

    #[test]
    fn a_dotted_path_reaches_a_nested_value() {
        assert_eq!(at(BODY, "data.order.id"), Some(serde_json::json!(7)));
    }

    #[test]
    fn a_numeric_segment_indexes_an_array() {
        assert_eq!(
            at(BODY, "data.order.items.0.sku"),
            Some(serde_json::json!("CHR-1"))
        );
    }

    #[test]
    fn a_path_that_leads_nowhere_is_none_rather_than_a_panic() {
        assert_eq!(at(BODY, "data.order.absent"), None);
        assert_eq!(at(BODY, "data.order.items.9.sku"), None);
        assert_eq!(at(BODY, "data.order.id.deeper"), None);
    }

    #[test]
    fn a_null_is_distinguishable_from_an_absent_key() {
        assert_eq!(
            at(BODY, "errors"),
            Some(serde_json::Value::Null),
            "GraphQL answers `\"errors\": null` on success. A case asserting on that must not be \
             told the key is missing"
        );
    }

    #[test]
    fn a_scalar_compares_as_text_so_the_shape_is_the_familiar_one() {
        assert!(
            check(&expecting("data.order.id", "7"), BODY, "expect").is_empty(),
            "`contains`/`absent` over the value rendered as text: a reader who knows `stdout` \
             knows this, and 7 matches \"7\" without the case minding JSON number types"
        );
    }

    #[test]
    fn a_string_compares_without_its_json_quotes() {
        assert!(
            check(
                &expecting("data.order.items.0.sku", "CHR-1"),
                BODY,
                "expect"
            )
            .is_empty(),
            "a case writing `CHR-1` must not have to write `\"CHR-1\"` with the quotes the \
             serialisation happens to use"
        );
    }

    #[test]
    fn a_body_that_is_not_json_reports_that_rather_than_a_missing_path() {
        let diffs = check(&expecting("data.id", "7"), "<html>500</html>", "expect");

        assert!(
            diffs[0].got.contains("not JSON"),
            "a case asserting on a JSON path against an HTML error page must be told the body was \
             not JSON. `path not found` would send the reader looking for a spelling mistake that \
             is not there: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn a_missing_path_says_which_keys_were_there() {
        let diffs = check(&expecting("data.order.idd", "7"), BODY, "expect");

        assert_eq!(diffs[0].path, "expect.json.data.order.idd");
        assert!(
            diffs[0].got.contains("id"),
            "listing the keys that *are* present turns a dead end into a next step, exactly as a \
             missing header does: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn a_wrong_value_shows_the_value_rather_than_saying_it_was_absent() {
        let diffs = check(&expecting("data.order.id", "9"), BODY, "expect");

        assert!(
            diffs[0].got.contains('7'),
            "a wrong value and a missing path are different failures, and a reader fixing one \
             needs to know which they have: {:?}",
            diffs[0].got
        );
    }
}
