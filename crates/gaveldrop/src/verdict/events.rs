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
                    got: nearest(want, actual, cursor),
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
    want.iter().all(|(key, value)| {
        event
            .fields
            .get(key)
            .is_some_and(|found| same(found, value))
    })
}

/// Whether two field values are the same, with `0` and `0.0` being the same number.
///
/// **JSON has one number type and its spellings are not interchangeable in `serde_json`.** A YAML
/// `cost: 0.0` deserialises to a float and a JSON `"cost": 0` parses to an integer, and the two are
/// unequal by derived equality — so a case asserting the cost of something free would never match,
/// against an event whose value is identical on any reading.
///
/// It is not a hypothetical spelling. `JSON.stringify(0.0)` in JavaScript emits `0` where
/// `serde_json` emits `0.0`, so which spelling reaches the case depends on what language the subject
/// is written in — something the person writing the case has no reason to be thinking about.
///
/// Integers are still compared exactly. Going through `f64` for two of them would make identifiers
/// past 2^53 compare equal when they are not, and a token count is a number where a cost is a
/// measurement.
///
/// `expect.json` deliberately does **not** do this and must not be changed to match. It renders the
/// value it found and applies text expectations to it, so a case writing `equals: "0"` against `0.0`
/// fails with `expected 0, got 0.0` — both spellings in front of the reader, understood at once.
/// Here there was nothing to see: a subset match either finds an event or does not.
fn same(found: &serde_json::Value, want: &serde_json::Value) -> bool {
    match (found.as_f64(), want.as_f64()) {
        (Some(left), Some(right)) if found.is_f64() || want.is_f64() => left == right,
        _ => found == want,
    }
}

/// Why the closest event was not it, or that nothing came close.
///
/// **A subsequence failure used to say only that nothing matched**, which is the least useful true
/// sentence available. The case it fails on is nearly always an event of the right type whose fields
/// carry different numbers — the subject emitted the `result` you asked for and the token count is
/// wrong — and a reader told "not found; 12 events observed" has to go and read all twelve.
///
/// The closest is the one sharing the most fields, so no configuration is needed: the type field is
/// one field among the others, and an event of the right type is already the one that shares most.
///
/// **An event that matched but too early is answered first.** A subsequence walks forward, so an
/// expectation whose event sits behind the cursor finds nothing ahead of it and used to be reported as
/// absent — sending the reader after a subject that never emitted it, which is precisely what the
/// evidence rules out. Reported by the first consumer's stress test, on a case that simply listed two
/// events the wrong way round.
fn nearest(want: &BTreeMap<String, serde_json::Value>, actual: &[Event], from: usize) -> String {
    // Before anything else, because "not found" would be false and misleading: the event is there, and
    // what is wrong is where. A reader told an event was missing goes looking for a subject that never
    // emitted it, which is the one explanation the evidence rules out.
    if let Some(at) = actual[..from]
        .iter()
        .position(|event| matches_partially(want, event))
    {
        return format!(
            "an event matching this is at position {}, before the previous expectation matched. \
             Events are checked in order, so one of the two lists is out of order — the case's or the \
             subject's",
            at + 1
        );
    }

    let scored = actual
        .iter()
        .enumerate()
        .skip(from)
        .map(|(at, event)| (agreement(want, event), at, event))
        .max_by_key(|(score, _, _)| *score);

    // Nothing after the cursor shares a single field, so there is no near miss to point at and the
    // honest answer is the plain one.
    let Some((_, at, event)) = scored.filter(|(score, _, _)| *score > 0) else {
        return format!(
            "not found after the previous match; {} events observed",
            actual.len()
        );
    };

    let differing: Vec<String> = want
        .iter()
        .filter(|(key, value)| {
            !event
                .fields
                .get(*key)
                .is_some_and(|found| same(found, value))
        })
        .map(|(key, value)| match event.fields.get(key) {
            Some(found) => format!("{key} is {found}, not {value}"),
            None => format!("{key} is absent"),
        })
        .collect();

    format!(
        "the closest was event {} of {}, where {}",
        at + 1,
        actual.len(),
        differing.join(" and ")
    )
}

/// How many of the wanted fields one event actually carries with the wanted value.
fn agreement(want: &BTreeMap<String, serde_json::Value>, event: &Event) -> usize {
    want.iter()
        .filter(|(key, value)| {
            event
                .fields
                .get(*key)
                .is_some_and(|found| same(found, value))
        })
        .count()
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

    /// One event with arbitrary fields, for the numeric and near-miss cases.
    fn raw(kind: &str, fields: &[(&str, serde_json::Value)]) -> Event {
        let mut all = BTreeMap::new();
        all.insert("t".to_string(), serde_json::json!(kind));
        for (key, value) in fields {
            all.insert((*key).to_string(), value.clone());
        }
        Event {
            kind: kind.to_string(),
            fields: all,
        }
    }

    /// One expectation with arbitrary fields, as a case would write it.
    fn asking(
        kind: &str,
        fields: &[(&str, serde_json::Value)],
    ) -> Vec<BTreeMap<String, serde_json::Value>> {
        let mut want = BTreeMap::new();
        want.insert("t".to_string(), serde_json::json!(kind));
        for (key, value) in fields {
            want.insert((*key).to_string(), value.clone());
        }
        vec![want]
    }

    /// The value a case wrote in YAML, parsed the way loading a case parses it.
    fn from_yaml(text: &str) -> serde_json::Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    /// A case asserting a cost of zero must match an event whose cost is zero.
    ///
    /// The trap: YAML `0.0` deserialises to a float and JSON `"cost": 0` parses to an integer, and
    /// `serde_json` calls those unequal. Which spelling reaches the case depends on what language the
    /// subject is written in — `JSON.stringify(0.0)` emits `0` where `serde_json` emits `0.0` — and
    /// nobody writing a case has a reason to be thinking about that.
    #[test]
    fn a_number_matches_whichever_way_the_two_sides_spell_it() {
        for (written, emitted) in [
            ("0.0", serde_json::json!(0)),
            ("0", serde_json::json!(0.0)),
            ("12", serde_json::json!(12.0)),
            ("12.0", serde_json::json!(12)),
            ("0.25", serde_json::json!(0.25)),
        ] {
            let actual = vec![raw("result", &[("cost", emitted.clone())])];
            let diffs =
                check_subsequence(&asking("result", &[("cost", from_yaml(written))]), &actual);

            assert!(
                diffs.is_empty(),
                "a case writing {written} against an emitted {emitted} is the same number: {diffs:?}"
            );
        }
    }

    /// Two integers are still compared exactly.
    ///
    /// Routing every comparison through `f64` would make identifiers past 2^53 compare equal when
    /// they are not. A token count is a number where a cost is a measurement, and only one of the two
    /// can afford to be rounded.
    #[test]
    fn two_large_integers_that_differ_do_not_become_equal() {
        let actual = vec![raw(
            "result",
            &[("id", serde_json::json!(9007199254740993u64))],
        )];
        let diffs = check_subsequence(
            &asking("result", &[("id", serde_json::json!(9007199254740992u64))]),
            &actual,
        );

        assert!(!diffs.is_empty(), "these are two different identifiers");
    }

    /// Numbers are forgiving about spelling, not about type.
    #[test]
    fn a_string_does_not_match_a_number() {
        let actual = vec![raw("result", &[("cost", serde_json::json!(0))])];
        let diffs = check_subsequence(
            &asking("result", &[("cost", serde_json::json!("0"))]),
            &actual,
        );

        assert!(
            !diffs.is_empty(),
            "a field that is text where the case expects a number is a real mismatch, and \
             coercing it would hide the day a subject started quoting its costs"
        );
    }

    /// The failure names the event that nearly matched, and what was wrong with it.
    ///
    /// "not found; 12 events observed" is the least useful true sentence available. The case it
    /// fails on is nearly always an event of the right type carrying a different number, and a
    /// reader given only a count has to go and read all twelve.
    #[test]
    fn a_subsequence_failure_points_at_the_closest_event() {
        let actual = vec![
            raw("run_start", &[]),
            raw(
                "result",
                &[
                    ("tin", serde_json::json!(6)),
                    ("agents", serde_json::json!(2)),
                ],
            ),
        ];
        let diffs = check_subsequence(
            &asking(
                "result",
                &[
                    ("tin", serde_json::json!(12)),
                    ("agents", serde_json::json!(2)),
                ],
            ),
            &actual,
        );

        assert_eq!(diffs.len(), 1);
        assert_eq!(
            diffs[0].got,
            "the closest was event 2 of 2, where tin is 6, not 12"
        );
    }

    /// A field the subject never emitted is named as absent rather than as a wrong value.
    #[test]
    fn a_field_the_closest_event_lacks_is_reported_as_absent() {
        let actual = vec![raw("result", &[("tin", serde_json::json!(12))])];
        let diffs = check_subsequence(
            &asking(
                "result",
                &[
                    ("tin", serde_json::json!(12)),
                    ("cost", serde_json::json!(0.0)),
                ],
            ),
            &actual,
        );

        assert_eq!(diffs.len(), 1);
        assert!(
            diffs[0].got.contains("cost is absent"),
            "a field that was never emitted is a different problem from a field with a wrong \
             value, and it usually means the case named it wrong: {}",
            diffs[0].got
        );
    }

    /// An event listed in the wrong order is not a missing event.
    ///
    /// The case the first consumer's stress test found: a subsequence walks forward, so an expectation
    /// whose event sits behind the cursor finds nothing ahead of it and was reported as absent. A reader
    /// told the event was missing goes looking for a subject that never emitted it — the one
    /// explanation the evidence rules out.
    #[test]
    fn an_event_that_matched_too_early_says_so_instead_of_saying_absent() {
        let actual = vec![raw("a", &[]), raw("b", &[])];

        let diffs = check_subsequence(
            &[asking("b", &[]).remove(0), asking("a", &[]).remove(0)],
            &actual,
        );

        assert_eq!(diffs.len(), 1);
        assert!(
            diffs[0]
                .got
                .contains("at position 1, before the previous expectation matched"),
            "where it is, not that it is missing: {}",
            diffs[0].got
        );
        assert!(
            diffs[0].got.contains("out of order"),
            "and what kind of mistake that is, since the case and the subject are both candidates: {}",
            diffs[0].got
        );
    }

    /// An event genuinely absent still says absent.
    #[test]
    fn an_event_nowhere_in_the_stream_is_still_reported_as_not_found() {
        let actual = vec![raw("a", &[]), raw("b", &[])];

        let diffs = check_subsequence(&asking("c", &[]), &actual);

        assert_eq!(diffs.len(), 1);
        assert_eq!(
            diffs[0].got, "not found after the previous match; 2 events observed",
            "the ordering sentence must not creep onto an event that really is not there"
        );
    }

    /// With nothing even close, the honest answer is the plain one.
    #[test]
    fn nothing_resembling_the_expectation_says_so_plainly() {
        let actual = vec![raw("run_start", &[]), raw("agent_start", &[])];
        let diffs = check_subsequence(&asking("result", &[]), &actual);

        assert_eq!(diffs.len(), 1);
        assert_eq!(
            diffs[0].got, "not found after the previous match; 2 events observed",
            "inventing a nearest event out of two that share no field would point the reader at \
             something irrelevant"
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
