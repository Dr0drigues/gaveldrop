//! The web adapter: start a service, interrogate it, stop it.
//!
//! The first subject in this project that does not run to completion. Everything else is invoked and
//! observed; a service has to be kept alive between exchanges and killed afterwards, which is why
//! `subject` exists.

pub mod request;
pub mod subject;

use std::time::Duration;

use gaveldrop_fake::FakeService;

use crate::adapters::{Adapter, AdapterError};
use crate::{Case, Isolation, Journal, Observations};

use subject::Subject;

/// How long a service gets to answer before its case fails.
///
/// Generous on purpose. A tight timeout tuned to a developer's machine is the classic way to make a
/// suite flaky on a loaded CI runner, and the cost of being generous is paid only by a case that was
/// going to fail anyway.
const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// Starts a service, performs each declared exchange against it, and stops it.
pub struct Web;

impl Adapter for Web {
    fn name(&self) -> &str {
        "web"
    }

    fn claims(&self, case: &Case) -> bool {
        case.setup.extra.contains_key("serve")
    }

    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError> {
        let argv = serve_command(case, iso)?;
        let mut subject =
            Subject::spawn(&argv, iso).map_err(|error| AdapterError::Unsupported {
                case: case.name.clone(),
                reason: error.to_string(),
            })?;

        let _faked = start_fake(case, iso)?;

        let (exit, steps) = if case.steps.is_empty() {
            (subject.wait_for_exit(), Vec::new())
        } else {
            wait_then_exchange(case, iso, &subject)?
        };

        let (stdout, stderr) = subject.output();

        Ok(Observations {
            exit,
            stdout,
            stderr,
            calls: Journal::read(&iso.journal_path())?,
            events: Vec::new(),
            files: iso.changes(),
            steps,
            ..Observations::default()
        })
    }
}

/// Waits for the service, then performs every declared exchange in order.
///
/// The exit code is `0` here on purpose: the subject is still running when the exchanges happen, and
/// reporting the code it will eventually be killed with would be inventing an observation. A case
/// asserting on a service's exit code is asserting on how gaveldrop stops it, which is not a property
/// of the subject.
fn wait_then_exchange(
    case: &Case,
    iso: &Isolation,
    subject: &Subject,
) -> Result<(i32, Vec<Observations>), AdapterError> {
    let defined = iso.defined();
    let probe = case
        .setup
        .extra
        .get("ready")
        .and_then(|value| value.as_str())
        .map(|declared| crate::iso::paths::expand_known(declared, &defined));

    subject
        .wait_until_ready(probe.as_deref(), READY_TIMEOUT)
        .map_err(|error| AdapterError::Unsupported {
            case: case.name.clone(),
            reason: error.to_string(),
        })?;

    let agent = agent();
    let port = port_of(iso, "GAVELDROP_PORT");
    let mut captured: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    let mut performed = Vec::with_capacity(case.steps.len());

    for step in &case.steps {
        let exchange = request::substituted(
            request::read(&step.request),
            &names_for(&captured, &defined),
        );
        let mut seen = request::perform(&agent, &exchange, port);
        capture_from(step, &mut seen, &mut captured);
        performed.push(seen);
    }

    Ok((0, performed))
}

/// The names a step may substitute: what earlier steps captured, with isolation's own on top.
///
/// Isolation wins, and that is checked rather than trusted. `HOME` means the isolated home in every
/// case ever written, and a document able to redefine it would hand the load-bearing invariant to
/// whoever writes the case.
fn names_for(
    captured: &std::collections::BTreeMap<String, String>,
    defined: &std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeMap<String, String> {
    let mut names = captured.clone();
    names.extend(
        defined
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    names
}

/// Names the values this step declared, for later steps to substitute.
///
/// A path that leads nowhere is **recorded** on the step's observations, for `verdict` to turn into
/// a failure at `capture.<name>`. The name also stays literal in the next request, so the case
/// fails there too — and the reader gets both halves in order: the path that found nothing, then
/// the request that went out without it.
///
/// Recorded in a field of its own rather than pushed onto the step's standard error, which is
/// where it used to go. That was writing into what we were measuring: the same field carries a
/// real failure — a request that could not be built — and a case is entitled to assert on it, so
/// our own commentary could satisfy or break an expectation it has no business touching.
fn capture_from(
    step: &crate::Step,
    seen: &mut Observations,
    into: &mut std::collections::BTreeMap<String, String>,
) {
    for (name, path) in &step.capture {
        // `null` counts as nothing found, not as the text "null". A path that leads to a null led to a
        // field the server chose not to fill, and substituting the four letters into the next request
        // sends out `/orders/null` — a 404 the reader investigates as a routing problem, three steps
        // away from the capture that caused it. The body is in the failure either way, so `"id": null`
        // is visible at a glance next to the path that found it.
        //
        // Only `null`. An empty string is a value the server chose to send, and refusing it would
        // refuse a legitimate capture of a field that is legitimately empty.
        match crate::verdict::json::at(&seen.body, path) {
            Some(value) if !value.is_null() => {
                into.insert(name.clone(), as_text(&value));
            }
            _ => {
                seen.missed_captures.insert(name.clone(), path.clone());
            }
        }
    }
}

/// A captured value as the text a later request substitutes.
fn as_text(value: &crate::Value) -> String {
    match value {
        crate::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The command line that starts the service, with the isolation's variables substituted.
///
/// Substituted because a service is usually a file of the project — `$GAVELDROP_PROJECT/app/server.py`
/// — and the subject runs with the isolated root as its working directory, where that file does not
/// exist.
///
/// A name isolation does not define is **left alone**, unlike a path in `expect.files`. A command is
/// very often a shell script, and `${MYVAR-default}` is that shell's syntax to read; refusing it
/// would reject a legitimate command for using a construct that was never ours to interpret.
fn serve_command(case: &Case, iso: &Isolation) -> Result<Vec<String>, AdapterError> {
    let declared = case
        .setup
        .extra
        .get("serve")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|argv| !argv.is_empty())
        .ok_or_else(|| AdapterError::Unsupported {
            case: case.name.clone(),
            reason: "setup has no `serve` command line naming the service to start".to_string(),
        })?;

    let defined = iso.defined();
    Ok(declared
        .iter()
        .map(|argument| crate::iso::paths::expand_known(argument, &defined))
        .collect())
}

/// Stands the faked service up, when the case has rules for it.
///
/// Returned so the caller holds it: dropping it stops the listener, and a fake outliving its case
/// would answer the next one's requests.
fn start_fake(case: &Case, iso: &Isolation) -> Result<Option<FakeService>, AdapterError> {
    let Some(scenario) = case.fake.clone() else {
        return Ok(None);
    };
    if scenario.rules.is_empty() {
        return Ok(None);
    }

    FakeService::start(
        scenario,
        iso.journal_path(),
        iso.root().join("state"),
        port_of(iso, "GAVELDROP_FAKE_PORT"),
    )
    .map(Some)
    .map_err(|error| AdapterError::Unsupported {
        case: case.name.clone(),
        reason: error.to_string(),
    })
}

/// A port the isolation reserved, or zero if it somehow did not.
fn port_of(iso: &Isolation, name: &str) -> u16 {
    iso.defined()
        .get(name)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

/// The client used for every exchange.
///
/// `http_status_as_error(false)` is what makes a 404 an observation rather than a failure: whether a
/// status is a problem is the case's decision, and an adapter never evaluates.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn step(capture: &[(&str, &str)]) -> crate::Step {
        crate::Step {
            name: None,
            request: BTreeMap::new(),
            expect: crate::Expect::default(),
            capture: capture
                .iter()
                .map(|(name, path)| ((*name).to_string(), (*path).to_string()))
                .collect(),
        }
    }

    fn answered(body: &str) -> Observations {
        Observations {
            body: body.to_string(),
            ..Observations::default()
        }
    }

    #[test]
    fn a_declared_capture_names_the_value_it_found() {
        let mut seen = answered(r#"{"data":{"order":{"id":7}}}"#);
        let mut captured = BTreeMap::new();

        capture_from(
            &step(&[("order_id", "data.order.id")]),
            &mut seen,
            &mut captured,
        );

        assert_eq!(captured.get("order_id"), Some(&"7".to_string()));
        assert!(
            seen.stderr.is_empty(),
            "a capture that worked must say nothing: {:?}",
            seen.stderr
        );
    }

    #[test]
    fn a_captured_string_loses_its_json_quotes() {
        let mut seen = answered(r#"{"sku":"CHR-1"}"#);
        let mut captured = BTreeMap::new();

        capture_from(&step(&[("sku", "sku")]), &mut seen, &mut captured);

        assert_eq!(
            captured.get("sku"),
            Some(&"CHR-1".to_string()),
            "a later request substitutes this into a path, where `\"CHR-1\"` with quotes would be a \
             different URL"
        );
    }

    /// A field the server left null is nothing captured, not the four letters "null".
    ///
    /// Found by auditing this path: `json::at` returns `Some(Null)` for a present-but-null field, so
    /// the capture used to succeed and substitute the text `null` into the next request. What the
    /// reader then saw was a 404 on `/orders/null`, three steps from the capture that caused it, and
    /// they would look at routing.
    ///
    /// Only `null`. An empty string is a value the server chose to send, and refusing it would refuse a
    /// legitimate capture of a field that is legitimately empty — asserted below, because the line
    /// between the two is the whole decision.
    #[test]
    fn a_capture_that_lands_on_null_is_nothing_captured() {
        let mut seen = answered(r#"{"id":null}"#);
        let mut captured = BTreeMap::new();

        capture_from(&step(&[("order_id", "id")]), &mut seen, &mut captured);

        assert!(
            captured.is_empty(),
            "substituting `null` would send out `/orders/null`: {captured:?}"
        );
        assert_eq!(
            seen.missed_captures.get("order_id").map(String::as_str),
            Some("id"),
            "and it is reported like any other capture that yielded no value — the body is in the \
             failure, so `\"id\":null` is visible beside the path that found it"
        );
    }

    /// An empty string is a value, and stays one.
    #[test]
    fn a_capture_that_lands_on_an_empty_string_is_still_a_capture() {
        let mut seen = answered(r#"{"note":""}"#);
        let mut captured = BTreeMap::new();

        capture_from(&step(&[("note", "note")]), &mut seen, &mut captured);

        assert_eq!(
            captured.get("note").map(String::as_str),
            Some(""),
            "the server chose to send an empty string; refusing it would refuse a field that is \
             legitimately empty"
        );
        assert!(seen.missed_captures.is_empty());
    }

    #[test]
    fn a_capture_that_finds_nothing_is_recorded_for_the_verdict_to_report() {
        let mut seen = answered(r#"{"data":null}"#);
        let mut captured = BTreeMap::new();

        capture_from(
            &step(&[("order_id", "data.order.id")]),
            &mut seen,
            &mut captured,
        );

        assert!(captured.is_empty());
        assert_eq!(
            seen.missed_captures.get("order_id").map(String::as_str),
            Some("data.order.id"),
            "the name and the path it was asked for, so `verdict` can fail the case at \
             `capture.order_id` — which is the half a reader was missing, since the other half is \
             a 404 a step later on a request carrying `$order_id` literally"
        );
    }

    #[test]
    fn nothing_is_written_into_the_stream_being_observed() {
        let mut seen = answered(r#"{"data":null}"#);
        let mut captured = BTreeMap::new();

        capture_from(
            &step(&[("order_id", "data.order.id")]),
            &mut seen,
            &mut captured,
        );

        assert!(
            seen.stderr.is_empty(),
            "this line used to go on the step's standard error. That field carries a real \
             failure — a request that could not be built — and a case may assert on it, so our \
             own commentary could satisfy a `contains` or break an `absent` that has nothing to \
             do with us: {:?}",
            seen.stderr
        );
    }

    #[test]
    fn an_isolation_variable_wins_over_a_capture_of_the_same_name() {
        let captured = BTreeMap::from([("HOME".to_string(), "/somewhere/else".to_string())]);
        let defined = BTreeMap::from([("HOME".to_string(), "/the/isolated/root".to_string())]);

        assert_eq!(
            names_for(&captured, &defined).get("HOME"),
            Some(&"/the/isolated/root".to_string()),
            "`HOME` means the isolated home in every case ever written. A document able to redefine \
             it would hand the load-bearing invariant to whoever writes the case"
        );
    }

    #[test]
    fn a_capture_is_available_beside_the_isolation_names_not_instead_of_them() {
        let captured = BTreeMap::from([("order_id".to_string(), "7".to_string())]);
        let defined = BTreeMap::from([("GAVELDROP_PORT".to_string(), "8080".to_string())]);
        let names = names_for(&captured, &defined);

        assert_eq!(names.get("order_id"), Some(&"7".to_string()));
        assert_eq!(names.get("GAVELDROP_PORT"), Some(&"8080".to_string()));
    }
}
