//! Extracting structured events from what the subject printed.
//!
//! By the placement rule, events belong to the core rather than to an extension: JSON lines
//! on standard output are observable of **any** process. A technology that emits them gets
//! event assertions for free; one that does not loses nothing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::verdict::Diff;

/// Where events are read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventSource {
    /// JSON objects, one per line, on standard output.
    #[default]
    StdoutJsonl,
}

/// How a project's events are recognised.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventsConfig {
    /// Which stream, in which shape.
    #[serde(default)]
    pub from: EventSource,
    /// The field naming the event's type. `t` in the prototype; anything in yours.
    pub type_field: String,
}

/// One structured event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// The value of the configured type field.
    pub kind: String,
    /// Every field of the object, the type field included.
    pub fields: BTreeMap<String, serde_json::Value>,
}

/// Reads events out of `stdout`.
///
/// A line that is not a JSON object, or that carries no string-valued type field, is **not an
/// event** and is skipped without complaint. A program mixing human output and structured
/// lines on one channel is the normal case, not an error — and structured logging that
/// happens to share the stream must not be mistaken for events.
pub fn extract(stdout: &str, config: &EventsConfig) -> Vec<Event> {
    stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok())
        .filter_map(|value| {
            let object = value.as_object()?;
            let kind = object.get(&config.type_field)?.as_str()?.to_string();
            Some(Event {
                kind,
                fields: object
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            })
        })
        .collect()
}

/// Checks that the expected events appear, in order, somewhere in `actual`.
///
/// A **subsequence**, not an exact list: a case names the events it cares about and tolerates
/// whatever else the subject emitted between them. Demanding an exact list would make every
/// case break the day the subject gains one new event, which is how event assertions get
/// deleted rather than maintained.
///
/// Returns at most one diff. Once the subsequence breaks, later positions mean nothing —
/// reporting them too would bury the one failure that matters.
pub fn check_subsequence(
    expected: &[BTreeMap<String, serde_json::Value>],
    actual: &[Event],
) -> Vec<Diff> {
    let mut cursor = 0;

    for (index, want) in expected.iter().enumerate() {
        match actual[cursor..]
            .iter()
            .position(|event| matches_partially(want, event))
        {
            Some(offset) => cursor += offset + 1,
            None => {
                return vec![Diff {
                    path: format!("expect.events[{index}]"),
                    expected: describe(want),
                    got: format!(
                        "not found after the previous match; {} events observed",
                        actual.len()
                    ),
                }];
            }
        }
    }

    Vec::new()
}

/// Checks how many events of each type were observed.
///
/// A declared `0` proves an event **never** happened — the graceful-degradation assertion that
/// says the budget warning was not emitted, the retry did not fire.
pub fn check_counts(expected: &BTreeMap<String, usize>, actual: &[Event]) -> Vec<Diff> {
    let mut diffs = Vec::new();

    for (kind, want) in expected {
        let got = actual.iter().filter(|event| &event.kind == kind).count();
        if got != *want {
            diffs.push(Diff {
                path: format!("expect.event_counts.{kind}"),
                expected: want.to_string(),
                got: got.to_string(),
            });
        }
    }

    diffs
}

/// True when every field the case named matches. Fields it did not name are not checked.
fn matches_partially(want: &BTreeMap<String, serde_json::Value>, event: &Event) -> bool {
    want.iter()
        .all(|(key, value)| event.fields.get(key) == Some(value))
}

/// A one-line rendering of a partial event, for a failure message.
fn describe(want: &BTreeMap<String, serde_json::Value>) -> String {
    let pairs: Vec<String> = want
        .iter()
        .map(|(key, value)| format!("{key}: {value}"))
        .collect();
    format!("{{ {} }}", pairs.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> EventsConfig {
        EventsConfig {
            from: EventSource::StdoutJsonl,
            type_field: "t".to_string(),
        }
    }

    #[test]
    fn json_lines_become_events_in_order() {
        let stdout = "{\"t\":\"run_start\",\"v\":1}\n{\"t\":\"agent_start\",\"agent\":\"alpha\"}\n";
        let events = extract(stdout, &config());

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "run_start");
        assert_eq!(events[1].fields["agent"], serde_json::json!("alpha"));
    }

    #[test]
    fn lines_that_are_not_json_are_ignored_rather_than_fatal() {
        let stdout = "starting up\n{\"t\":\"result\"}\nall done\n";
        let events = extract(stdout, &config());

        assert_eq!(
            events.len(),
            1,
            "a program that mixes human output with structured lines is the normal case, not \
             an error"
        );
        assert_eq!(events[0].kind, "result");
    }

    #[test]
    fn a_json_line_without_the_type_field_is_not_an_event() {
        let stdout = "{\"level\":\"info\",\"msg\":\"hello\"}\n{\"t\":\"result\"}\n";
        let events = extract(stdout, &config());

        assert_eq!(
            events.len(),
            1,
            "structured logging output shares the channel; only lines carrying the configured \
             type field are events"
        );
    }

    #[test]
    fn the_type_field_name_comes_from_the_configuration() {
        let events = extract(
            "{\"event\":\"result\"}\n",
            &EventsConfig {
                from: EventSource::StdoutJsonl,
                type_field: "event".to_string(),
            },
        );

        assert_eq!(events[0].kind, "result");
    }

    #[test]
    fn a_non_string_type_is_not_an_event() {
        assert!(extract("{\"t\":42}\n", &config()).is_empty());
    }

    #[test]
    fn a_json_array_or_scalar_line_is_not_an_event() {
        assert!(extract("[1,2,3]\n\"just a string\"\n7\n", &config()).is_empty());
    }

    #[test]
    fn the_type_field_is_kept_in_the_fields_too() {
        let events = extract("{\"t\":\"result\",\"n\":1}\n", &config());

        assert_eq!(
            events[0].fields["t"],
            serde_json::json!("result"),
            "an `events` assertion is written as a partial object including `t`, so the field \
             must be matchable like any other"
        );
    }

    fn event(kind: &str, agent: Option<&str>) -> Event {
        let mut fields = BTreeMap::new();
        fields.insert("t".to_string(), serde_json::json!(kind));
        if let Some(agent) = agent {
            fields.insert("agent".to_string(), serde_json::json!(agent));
        }
        Event {
            kind: kind.to_string(),
            fields,
        }
    }

    fn wanted(pairs: &[(&str, Option<&str>)]) -> Vec<BTreeMap<String, serde_json::Value>> {
        pairs
            .iter()
            .map(|(kind, agent)| {
                let mut want = BTreeMap::new();
                want.insert("t".to_string(), serde_json::json!(kind));
                if let Some(agent) = agent {
                    want.insert("agent".to_string(), serde_json::json!(agent));
                }
                want
            })
            .collect()
    }

    #[test]
    fn expected_events_are_matched_as_a_subsequence_not_an_exact_list() {
        let actual = vec![
            event("run_start", None),
            event("agent_start", Some("alpha")),
            event("noise", None),
            event("result", None),
        ];
        let diffs = check_subsequence(&wanted(&[("run_start", None), ("result", None)]), &actual);

        assert!(
            diffs.is_empty(),
            "a case names the events it cares about, in order, and tolerates others in \
             between: diffs {diffs:?}"
        );
    }

    #[test]
    fn events_out_of_order_fail_and_the_diff_names_the_one_that_was_missed() {
        let actual = vec![event("result", None), event("run_start", None)];
        let diffs = check_subsequence(&wanted(&[("run_start", None), ("result", None)]), &actual);

        assert_eq!(
            diffs.len(),
            1,
            "once the subsequence breaks, later positions mean nothing: reporting them too \
             would bury the one failure that matters"
        );
        assert_eq!(diffs[0].path, "expect.events[1]");
        assert!(diffs[0].expected.contains("result"));
    }

    #[test]
    fn a_partial_object_only_constrains_the_fields_it_names() {
        let actual = vec![event("agent_start", Some("alpha"))];

        assert!(check_subsequence(&wanted(&[("agent_start", None)]), &actual).is_empty());
        assert!(
            !check_subsequence(&wanted(&[("agent_start", Some("bravo"))]), &actual).is_empty(),
            "a field the case does name must match"
        );
    }

    #[test]
    fn the_same_event_twice_needs_two_occurrences() {
        let once = vec![event("vote", None)];
        assert!(
            !check_subsequence(&wanted(&[("vote", None), ("vote", None)]), &once).is_empty(),
            "the cursor must advance past each match, or one event would satisfy every \
             expectation naming it"
        );

        let twice = vec![event("vote", None), event("vote", None)];
        assert!(check_subsequence(&wanted(&[("vote", None), ("vote", None)]), &twice).is_empty());
    }

    #[test]
    fn counts_are_checked_per_type() {
        let actual = vec![
            event("agent_start", Some("alpha")),
            event("agent_start", Some("bravo")),
            event("result", None),
        ];
        let expected = [("agent_start".to_string(), 2), ("result".to_string(), 1)]
            .into_iter()
            .collect();

        assert!(check_counts(&expected, &actual).is_empty());
    }

    #[test]
    fn a_count_that_is_off_names_the_type_in_its_path() {
        let actual = vec![event("result", None), event("result", None)];
        let expected = [("result".to_string(), 1)].into_iter().collect();

        let diffs = check_counts(&expected, &actual);
        assert_eq!(diffs[0].path, "expect.event_counts.result");
        assert_eq!(diffs[0].expected, "1");
        assert_eq!(diffs[0].got, "2");
    }

    #[test]
    fn a_declared_count_of_zero_proves_an_event_never_happened() {
        let expected: BTreeMap<String, usize> =
            [("budget_exceeded".to_string(), 0)].into_iter().collect();

        assert!(check_counts(&expected, &[event("result", None)]).is_empty());
        assert!(
            !check_counts(&expected, &[event("budget_exceeded", None)]).is_empty(),
            "asserting an event never happened is the graceful-degradation check: the retry \
             did not fire, the budget warning was not emitted"
        );
    }
}
