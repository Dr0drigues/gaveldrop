//! Extracting structured events from what the subject printed.
//!
//! By the placement rule, events belong to the core rather than to an extension: JSON lines
//! on standard output are observable of **any** process. A technology that emits them gets
//! event assertions for free; one that does not loses nothing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
}
