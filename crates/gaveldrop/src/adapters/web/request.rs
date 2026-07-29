//! Performing one exchange with a living service.

use std::collections::BTreeMap;

use crate::{Observations, Value};

/// What one step asked for, read out of its opaque `request:` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange {
    /// The HTTP method, uppercased. `GET` when the step does not say.
    pub method: String,
    /// The path, always starting with `/`.
    pub path: String,
    /// The request body, empty when there is none.
    pub body: String,
    /// Headers to send.
    pub headers: BTreeMap<String, String>,
}

/// Reads a step's `request:` block.
///
/// A **path** rather than a full URL, because the port is not knowable when the case is written — it
/// is chosen per run. Writing `http://127.0.0.1:$GAVELDROP_PORT/orders` by hand would put
/// interpolation into the format for no gain, and the format holds no logic on purpose.
///
/// Everything is optional. A step that says nothing performs `GET /`, which is what a readiness or
/// smoke exchange looks like.
pub fn read(request: &BTreeMap<String, Value>) -> Exchange {
    Exchange {
        method: string(request, "method")
            .unwrap_or_else(|| "GET".to_string())
            .to_uppercase(),
        path: normalise(&string(request, "path").unwrap_or_else(|| "/".to_string())),
        body: body_of(request),
        headers: map(request, "headers"),
    }
}

/// The same exchange with `names` substituted into its path, body and headers.
///
/// Applied after [`read`] rather than inside it, so what a step declared and what was sent are two
/// values a reader can compare. Unknown names are left literal — a case whose capture found nothing
/// then requests `/orders/$order_id`, which fails visibly, rather than `/orders/` which would fail
/// like the service's own bug.
pub fn substituted(exchange: Exchange, names: &BTreeMap<String, String>) -> Exchange {
    Exchange {
        method: exchange.method,
        path: crate::iso::paths::expand_known(&exchange.path, names),
        body: crate::iso::paths::expand_known(&exchange.body, names),
        headers: exchange
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), crate::iso::paths::expand_known(value, names)))
            .collect(),
    }
}

/// The URL to call for `exchange` on the service listening at `port`.
pub fn url_for(exchange: &Exchange, port: u16) -> String {
    format!("http://127.0.0.1:{port}{}", exchange.path)
}

/// Performs `exchange` and reports what came back.
///
/// A transport failure — connection refused, a timeout — becomes an `Observations` with no status
/// rather than an error. The subject was running and this exchange did not get an answer, which is a
/// failed expectation about that exchange, not a broken run. `expect.status` then reports
/// `no response at all`.
pub fn perform(agent: &ureq::Agent, exchange: &Exchange, port: u16) -> Observations {
    let url = url_for(exchange, port);

    let mut building = ureq::http::Request::builder()
        .method(exchange.method.as_str())
        .uri(&url);
    for (name, value) in &exchange.headers {
        building = building.header(name, value);
    }

    let built = match building.body(exchange.body.as_str()) {
        Ok(request) => request,
        Err(error) => {
            return Observations {
                stderr: format!("building the request for {url}: {error}"),
                ..Observations::default()
            };
        }
    };

    match agent.run(built) {
        Ok(mut response) => Observations {
            status: Some(response.status().as_u16()),
            headers: response
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|value| (name.to_string(), value.to_string()))
                })
                .collect(),
            body: response.body_mut().read_to_string().unwrap_or_default(),
            ..Observations::default()
        },
        Err(error) => Observations {
            stderr: format!("requesting {url}: {error}"),
            ..Observations::default()
        },
    }
}

/// A leading slash, added when the case left it out.
fn normalise(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

/// A string value from the opaque block.
fn string(request: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    request
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

/// The body, whether the case wrote it as a string or as a JSON object.
///
/// A case testing a JSON API should not have to escape quotes inside a YAML string. Written as a
/// mapping it is serialised back to JSON here, which is both easier to write and easier to read.
fn body_of(request: &BTreeMap<String, Value>) -> String {
    match request.get("body") {
        None => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// A string map from the opaque block.
fn map(request: &BTreeMap<String, Value>, key: &str) -> BTreeMap<String, String> {
    request
        .get(key)
        .and_then(|value| value.as_object())
        .map(|fields| {
            fields
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .as_str()
                        .map(|value| (name.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn block(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn a_step_that_says_nothing_performs_a_get_on_the_root() {
        let exchange = read(&BTreeMap::new());

        assert_eq!(exchange.method, "GET");
        assert_eq!(
            exchange.path, "/",
            "a smoke exchange should be writable as `- expect: {{ status: 200 }}` with no request \
             block at all"
        );
    }

    #[test]
    fn a_lowercase_method_is_accepted() {
        assert_eq!(read(&block(&[("method", json!("post"))])).method, "POST");
    }

    #[test]
    fn a_path_without_its_leading_slash_still_works() {
        assert_eq!(
            read(&block(&[("path", json!("orders"))])).path,
            "/orders",
            "forgetting the slash is a typo, not a different intention"
        );
    }

    #[test]
    fn a_body_written_as_a_mapping_is_sent_as_json() {
        let exchange = read(&block(&[("body", json!({"item": "chair", "qty": 2}))]));

        assert_eq!(
            exchange.body, "{\"item\":\"chair\",\"qty\":2}",
            "a case testing a JSON API should not have to escape quotes inside a YAML string"
        );
    }

    #[test]
    fn a_body_written_as_a_string_is_sent_verbatim() {
        let exchange = read(&block(&[("body", json!("not json at all"))]));

        assert_eq!(
            exchange.body, "not json at all",
            "an API taking plain text or a form must stay testable"
        );
    }

    #[test]
    fn the_url_joins_the_chosen_port_to_the_declared_path() {
        let exchange = read(&block(&[("path", json!("/orders/7"))]));

        assert_eq!(
            url_for(&exchange, 54321),
            "http://127.0.0.1:54321/orders/7",
            "the case declares a path and the run supplies the port, so no interpolation enters \
             the format"
        );
    }

    fn names(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn a_captured_name_is_substituted_into_the_path() {
        let exchange = read(&block(&[("path", json!("/orders/$order_id"))]));

        assert_eq!(
            substituted(exchange, &names(&[("order_id", "7")])).path,
            "/orders/7",
            "this is the whole point of a capture: one exchange creates a thing and the next asks \
             about it by the id it was given"
        );
    }

    #[test]
    fn a_captured_name_is_substituted_into_the_body_and_headers() {
        let exchange = read(&block(&[
            ("body", json!({"order": "$order_id"})),
            ("headers", json!({"X-Trace": "$trace"})),
        ]));
        let sent = substituted(exchange, &names(&[("order_id", "7"), ("trace", "abc")]));

        assert!(sent.body.contains("\"7\""), "got {:?}", sent.body);
        assert_eq!(sent.headers.get("X-Trace"), Some(&"abc".to_string()));
    }

    #[test]
    fn a_name_nothing_captured_is_left_literal_so_the_failure_is_visible() {
        let exchange = read(&block(&[("path", json!("/orders/$never_captured"))]));

        assert_eq!(
            substituted(exchange, &names(&[])).path,
            "/orders/$never_captured",
            "requesting `/orders/` instead would produce a 404 that reads like the service's own \
             bug. Leaving the name in makes the cause visible in the report"
        );
    }

    #[test]
    fn a_transport_failure_becomes_an_absent_status_rather_than_an_error() {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_millis(200)))
            .http_status_as_error(false)
            .build()
            .into();
        let observed = perform(&agent, &read(&BTreeMap::new()), 1);

        assert!(
            observed.status.is_none(),
            "the subject was running and this exchange got no answer. That is a failed \
             expectation about the exchange, not a broken run, and `expect.status` reports it as \
             `no response at all`"
        );
        assert!(
            observed.stderr.contains("127.0.0.1:1"),
            "and it must say what it tried to reach: {:?}",
            observed.stderr
        );
    }
}
