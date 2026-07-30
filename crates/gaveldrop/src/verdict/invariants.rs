//! Named invariants: four shapes, no library.
//!
//! Four because those are exactly the ones the prototype had, and a speculative invariant
//! library would be dead weight. A fifth shape gets added the day a real case demands one.
//!
//! The shapes are parameterised, not hard-coded: a project names them in its configuration and
//! a case uses the name. That is what makes an invariant written once serve everywhere without
//! the core learning any project's event vocabulary.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::verdict::Diff;
use crate::verdict::events::Event;

/// One of the four shapes an invariant can take.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
pub enum InvariantShape {
    /// Every `start` has an `end` carrying the same `key` value, and the counts agree.
    Paired {
        /// Event type that opens.
        start: String,
        /// Event type that closes.
        end: String,
        /// Field identifying which one closes which.
        key: String,
    },
    /// Exactly one event of this type.
    ExactlyOne {
        /// The type there must be one of.
        #[serde(rename = "type")]
        kind: String,
    },
    /// Every event of this type carries `field`, present and non-empty.
    ///
    /// **One field, deliberately.** A project wanting two — "every `agent_start` carries both a
    /// provider and a model" — declares two named invariants rather than one taking a list. It
    /// costs a line of configuration and buys the diagnostic: a case failing
    /// `model_non_empty` says which of the two was missing, where a
    /// `prov_and_model_non_empty` would only say that one of them was. Naming what broke is the
    /// third property, and a list would trade it away for the shorter configuration.
    FieldNonEmpty {
        /// The type to look at.
        #[serde(rename = "type")]
        kind: String,
        /// The field that must be filled in.
        field: String,
    },
    /// Every event carrying `key` was preceded by a `root` event with the same value.
    NoOrphan {
        /// The field being referenced.
        key: String,
        /// The event type that declares a key.
        root: String,
    },
}

/// A project's named invariants, as its configuration declares them.
pub type NamedInvariants = BTreeMap<String, InvariantShape>;

/// Checks one invariant, returning a diff when it does not hold.
///
/// Located by `name` — the name the project gave it, which is what the case wrote. A failure
/// saying "the paired shape failed" would send the reader to the configuration to work out
/// which one.
pub fn check(shape: &InvariantShape, name: &str, events: &[Event]) -> Option<Diff> {
    let failure = match shape {
        InvariantShape::Paired { start, end, key } => paired(events, start, end, key),
        InvariantShape::ExactlyOne { kind } => exactly_one(events, kind),
        InvariantShape::FieldNonEmpty { kind, field } => field_non_empty(events, kind, field),
        InvariantShape::NoOrphan { key, root } => no_orphan(events, key, root),
    }?;

    Some(Diff {
        path: format!("expect.invariants.{name}"),
        expected: "holds".to_string(),
        got: failure,
    })
}

/// The key values that opened without closing, or closed without opening.
fn paired(events: &[Event], start: &str, end: &str, key: &str) -> Option<String> {
    let opened = keys_of(events, start, key);
    let closed = keys_of(events, end, key);

    let dangling: Vec<&String> = opened.difference(&closed).collect();
    let orphaned: Vec<&String> = closed.difference(&opened).collect();

    if dangling.is_empty() && orphaned.is_empty() {
        return None;
    }

    let mut parts = Vec::new();
    if !dangling.is_empty() {
        parts.push(format!("{start} without {end}: {dangling:?}"));
    }
    if !orphaned.is_empty() {
        parts.push(format!("{end} without {start}: {orphaned:?}"));
    }
    Some(parts.join("; "))
}

/// A complaint unless there is exactly one event of `kind`.
fn exactly_one(events: &[Event], kind: &str) -> Option<String> {
    let count = events.iter().filter(|event| event.kind == kind).count();
    (count != 1).then(|| format!("{count} events of type {kind}, expected exactly one"))
}

/// The number of events of `kind` whose `field` was missing or empty.
fn field_non_empty(events: &[Event], kind: &str, field: &str) -> Option<String> {
    let offenders = events
        .iter()
        .filter(|event| event.kind == kind)
        .filter(|event| !is_filled(event.fields.get(field)))
        .count();

    (offenders > 0).then(|| format!("{offenders} {kind} events with {field} missing or empty"))
}

/// The key values used without a preceding `root` event.
///
/// Order is the point: a key used before it was declared is the bug this shape exists to
/// catch, so the walk is sequential rather than a set comparison.
fn no_orphan(events: &[Event], key: &str, root: &str) -> Option<String> {
    let mut declared: BTreeSet<String> = BTreeSet::new();
    let mut orphans: BTreeSet<String> = BTreeSet::new();

    for event in events {
        let Some(value) = event.fields.get(key).and_then(|value| value.as_str()) else {
            continue;
        };

        if event.kind == root {
            declared.insert(value.to_string());
        } else if !declared.contains(value) {
            orphans.insert(value.to_string());
        }
    }

    (!orphans.is_empty()).then(|| format!("used before any {root}: {orphans:?}"))
}

/// The distinct string values of `key` across events of `kind`.
fn keys_of(events: &[Event], kind: &str, key: &str) -> BTreeSet<String> {
    events
        .iter()
        .filter(|event| event.kind == kind)
        .filter_map(|event| event.fields.get(key)?.as_str())
        .map(String::from)
        .collect()
}

/// True when a field is present and not an empty string.
fn is_filled(value: Option<&serde_json::Value>) -> bool {
    match value {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::String(text)) => !text.is_empty(),
        Some(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: &str, fields: &[(&str, serde_json::Value)]) -> Event {
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

    fn paired() -> InvariantShape {
        InvariantShape::Paired {
            start: "agent_start".to_string(),
            end: "agent_end".to_string(),
            key: "agent".to_string(),
        }
    }

    #[test]
    fn paired_accepts_a_matched_pair_and_rejects_a_dangling_start() {
        let matched = vec![
            event("agent_start", &[("agent", serde_json::json!("alpha"))]),
            event("agent_end", &[("agent", serde_json::json!("alpha"))]),
        ];
        assert!(check(&paired(), "agent_start_end_symmetric", &matched).is_none());

        let dangling = vec![event(
            "agent_start",
            &[("agent", serde_json::json!("alpha"))],
        )];
        let diff = check(&paired(), "agent_start_end_symmetric", &dangling).unwrap();

        assert_eq!(
            diff.path, "expect.invariants.agent_start_end_symmetric",
            "an invariant failure is located by the name the project gave it, which is what \
             the case wrote"
        );
        assert!(diff.got.contains("alpha"));
    }

    #[test]
    fn paired_rejects_an_end_whose_key_never_started() {
        let orphaned = vec![event("agent_end", &[("agent", serde_json::json!("ghost"))])];
        assert!(check(&paired(), "symmetric", &orphaned).is_some());
    }

    #[test]
    fn exactly_one_rejects_both_zero_and_two() {
        let shape = InvariantShape::ExactlyOne {
            kind: "result".to_string(),
        };

        assert!(check(&shape, "single_result", &[event("result", &[])]).is_none());
        assert!(check(&shape, "single_result", &[]).is_some());
        assert!(
            check(
                &shape,
                "single_result",
                &[event("result", &[]), event("result", &[])]
            )
            .is_some()
        );
    }

    #[test]
    fn field_non_empty_rejects_a_missing_field_and_an_empty_string() {
        let shape = InvariantShape::FieldNonEmpty {
            kind: "provider".to_string(),
            field: "model".to_string(),
        };

        let good = vec![event("provider", &[("model", serde_json::json!("m-1"))])];
        assert!(check(&shape, "prov_model_non_empty", &good).is_none());

        let empty = vec![event("provider", &[("model", serde_json::json!(""))])];
        assert!(check(&shape, "prov_model_non_empty", &empty).is_some());

        let absent = vec![event("provider", &[])];
        assert!(check(&shape, "prov_model_non_empty", &absent).is_some());
    }

    #[test]
    fn field_non_empty_ignores_events_of_other_types() {
        let shape = InvariantShape::FieldNonEmpty {
            kind: "provider".to_string(),
            field: "model".to_string(),
        };
        assert!(check(&shape, "prov", &[event("result", &[])]).is_none());
    }

    #[test]
    fn no_orphan_requires_a_root_event_before_any_use_of_the_key() {
        let shape = InvariantShape::NoOrphan {
            key: "agent".to_string(),
            root: "agent_start".to_string(),
        };

        let ordered = vec![
            event("agent_start", &[("agent", serde_json::json!("alpha"))]),
            event("vote", &[("agent", serde_json::json!("alpha"))]),
        ];
        assert!(check(&shape, "no_orphan_events", &ordered).is_none());

        let orphan = vec![event("vote", &[("agent", serde_json::json!("ghost"))])];
        let diff = check(&shape, "no_orphan_events", &orphan).unwrap();
        assert!(
            diff.got.contains("ghost"),
            "the failure must name the key that had no root, or the reader has to diff two \
             event streams by hand"
        );
    }

    #[test]
    fn no_orphan_rejects_a_root_that_arrives_too_late() {
        let shape = InvariantShape::NoOrphan {
            key: "agent".to_string(),
            root: "agent_start".to_string(),
        };
        let late = vec![
            event("vote", &[("agent", serde_json::json!("alpha"))]),
            event("agent_start", &[("agent", serde_json::json!("alpha"))]),
        ];

        assert!(
            check(&shape, "no_orphan_events", &late).is_some(),
            "order is the point: a key used before it was declared is the bug this shape \
             exists to catch"
        );
    }

    #[test]
    fn every_shape_parses_from_the_yaml_a_project_would_write() {
        let yaml = r#"
agent_start_end_symmetric: { shape: paired, start: agent_start, end: agent_end, key: agent }
single_result:             { shape: exactly_one, type: result }
prov_model_non_empty:      { shape: field_non_empty, type: provider, field: model }
no_orphan_events:          { shape: no_orphan, key: agent, root: agent_start }
"#;
        let declared: NamedInvariants = serde_yaml_ng::from_str(yaml).unwrap();

        assert_eq!(
            declared.len(),
            4,
            "four shapes, exactly the ones the prototype had: a speculative invariant library \
             would be dead weight"
        );
    }

    #[test]
    fn an_unknown_shape_is_refused_rather_than_ignored() {
        let error = serde_yaml_ng::from_str::<NamedInvariants>("weird: { shape: telepathy }\n")
            .unwrap_err();

        assert!(
            error.to_string().contains("telepathy") || error.to_string().contains("paired"),
            "a misspelled shape must fail at load time: silently skipping it would turn a typo \
             into an invariant that checks nothing. Got: {error}"
        );
    }
}
