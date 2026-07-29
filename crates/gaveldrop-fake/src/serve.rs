//! The rule engine behind an HTTP door.
//!
//! The binary door exists out of necessity: a subject finds a faked tool by name on `PATH`, and only
//! an executable can be found that way. A faked *service* has no such constraint, since gaveldrop
//! starts it. So this is a thread using the engine as a library — no second binary to ship, no
//! start-up handshake between processes.
//!
//! **The rules are the same rules.** A project writes `fake.rules` once, and whether the dependency
//! arrives as an executable on `PATH` or a request on a port changes the door, never the matching,
//! the counter or the journal. If the two doors ever needed different rules, the engine would be
//! wrong.

use std::path::PathBuf;
use std::sync::Arc;

use crate::{Call, Counter, Invocation, Journal, Scenario};

/// A faked service, listening until it is dropped.
pub struct FakeService {
    server: Arc<tiny_http::Server>,
    port: u16,
}

/// What can go wrong standing a faked service up.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The port could not be listened on.
    #[error("listening on port {port} for the faked service: {reason}")]
    Listen {
        /// The port asked for.
        port: u16,
        /// What the listener said.
        reason: String,
    },
    /// The scenario asks for a response mode this door cannot honour.
    ///
    /// Named at start-up rather than at the first request. A scenario that cannot work should say so
    /// before the subject is running, not fail obscurely once it is.
    #[error(
        "the HTTP door cannot honour `{mode}`: {why}. Use `stdout` with `status` and `headers`, \
         which is what a faked service answers with"
    )]
    UnsupportedMode {
        /// The mode the scenario asked for.
        mode: String,
        /// Why this door cannot do it.
        why: String,
    },
}

impl FakeService {
    /// Starts listening on `port`, answering every request from `scenario`.
    ///
    /// Pass `0` to let the kernel choose and read it back from [`FakeService::port`], which is what
    /// avoids the reserve-then-bind race a fixed port would reintroduce.
    pub fn start(
        scenario: Scenario,
        journal: PathBuf,
        counter_dir: PathBuf,
        port: u16,
    ) -> Result<Self, ServeError> {
        refuse_unsupported(&scenario)?;

        let server =
            tiny_http::Server::http(("127.0.0.1", port)).map_err(|error| ServeError::Listen {
                port,
                reason: error.to_string(),
            })?;
        let port = server
            .server_addr()
            .to_ip()
            .map(|address| address.port())
            .unwrap_or(port);

        let server = Arc::new(server);
        let answering = Arc::clone(&server);

        std::thread::spawn(move || {
            for request in answering.incoming_requests() {
                answer(request, &scenario, &journal, &counter_dir);
            }
        });

        Ok(Self { server, port })
    }

    /// The port it is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for FakeService {
    /// Stops accepting, which ends the answering thread.
    fn drop(&mut self) {
        self.server.unblock();
    }
}

/// Refuses a scenario whose modes this door has no meaning for.
fn refuse_unsupported(scenario: &Scenario) -> Result<(), ServeError> {
    if scenario.render.is_some() {
        return Err(ServeError::UnsupportedMode {
            mode: "render".to_string(),
            why: "shaping bytes through a hook means capturing its output, which the binary door \
                  does by inheriting its own streams"
                .to_string(),
        });
    }

    if let Some(rule) = scenario
        .rules
        .iter()
        .find(|rule| rule.response.exec.is_some())
    {
        let mode = rule.response.exec.clone().unwrap_or_default();
        return Err(ServeError::UnsupportedMode {
            mode: format!("exec: {mode}"),
            why: "`exec: real` finds the next binary along `PATH`, and there is no next service \
                  along a port"
                .to_string(),
        });
    }

    Ok(())
}

/// Answers one request from the rules, and journals it.
///
/// A failure to journal is swallowed on purpose: the subject is mid-request, and a panic in this
/// thread would leave it hanging on a connection that never answers. The missing line shows up as a
/// count that does not add up, which is a failed case rather than a hung suite.
fn answer(
    mut request: tiny_http::Request,
    scenario: &Scenario,
    journal: &PathBuf,
    counter_dir: &PathBuf,
) {
    let invocation = as_invocation(&mut request);
    let key = invocation.bin.clone();
    let call = Counter::new(counter_dir).next(&key).unwrap_or(1);

    let Some(rule) = scenario.select(&invocation, call) else {
        let _ = request.respond(tiny_http::Response::empty(500));
        return;
    };

    let status = rule.response.status.unwrap_or(200);
    let body = rule.response.stdout.clone().unwrap_or_default();

    if let Some(wait) = rule.response.latency_ms {
        std::thread::sleep(std::time::Duration::from_millis(wait));
    }

    let mut response = tiny_http::Response::from_string(body).with_status_code(status);
    for (name, value) in &rule.response.headers {
        if let Ok(header) = tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response = response.with_header(header);
        }
    }

    let _ = Journal::new(journal).record(&Call::from_invocation(
        &invocation,
        call,
        &key,
        rule.matcher.is_catch_all(),
        false,
        i32::from(status),
    ));

    let _ = request.respond(response);
}

/// Turns a request into the shape the rule engine already matches on.
///
/// The path becomes `bin`, so the counter key is the path and `call: 2` means the second request to
/// it. The method and query become arguments, which is what lets `args_contain: POST` read the same
/// way it reads a binary's flags. The body becomes `stdin`, so `stdin_contains` works unchanged.
fn as_invocation(request: &mut tiny_http::Request) -> Invocation {
    let url = request.url().to_string();
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path.to_string(), Some(query.to_string())),
        None => (url, None),
    };

    let mut args = vec![request.method().as_str().to_string()];
    args.extend(query);

    let mut stdin = String::new();
    let _ = std::io::Read::read_to_string(request.as_reader(), &mut stdin);

    Invocation {
        bin: path,
        args,
        stdin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{Journal, Match, Response, Rule, Scenario};

    fn scenario(rules: Vec<Rule>) -> Scenario {
        Scenario {
            render: None,
            rules,
        }
    }

    fn rule(matcher: Match, response: Response) -> Rule {
        Rule { matcher, response }
    }

    fn catch_all(body: &str) -> Rule {
        rule(
            Match::default(),
            Response {
                stdout: Some(body.to_string()),
                status: Some(500),
                ..Response::default()
            },
        )
    }

    struct Fixture {
        service: FakeService,
        journal: Journal,
        _root: tempfile::TempDir,
    }

    fn serving(scenario: Scenario) -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let journal = Journal::new(root.path().join("journal.jsonl"));
        let service = FakeService::start(
            scenario,
            root.path().join("journal.jsonl"),
            root.path().to_path_buf(),
            0,
        )
        .unwrap();

        Fixture {
            service,
            journal,
            _root: root,
        }
    }

    fn get(fixture: &Fixture, path: &str) -> (u16, String) {
        let url = format!("http://127.0.0.1:{}{path}", fixture.service.port());
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let mut response = agent.get(&url).call().unwrap();
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_string().unwrap();
        (status, body)
    }

    #[test]
    fn a_matching_request_is_answered_from_the_scenario() {
        let fixture = serving(scenario(vec![
            rule(
                Match {
                    bin: Some("/orders".to_string()),
                    ..Match::default()
                },
                Response {
                    stdout: Some("{\"id\":7}".to_string()),
                    status: Some(201),
                    ..Response::default()
                },
            ),
            catch_all("unforeseen"),
        ]));

        assert_eq!(get(&fixture, "/orders"), (201, "{\"id\":7}".to_string()));
    }

    #[test]
    fn a_status_the_rule_leaves_out_defaults_to_two_hundred() {
        let fixture = serving(scenario(vec![rule(
            Match::default(),
            Response {
                stdout: Some("fine".to_string()),
                ..Response::default()
            },
        )]));

        assert_eq!(
            get(&fixture, "/anything").0,
            200,
            "a rule that says nothing about status must answer 200, so a project faking a happy \
             path writes one line"
        );
    }

    #[test]
    fn declared_headers_reach_the_client() {
        let fixture = serving(scenario(vec![rule(
            Match::default(),
            Response {
                stdout: Some("{}".to_string()),
                headers: std::collections::BTreeMap::from([(
                    "Content-Type".to_string(),
                    "application/json".to_string(),
                )]),
                ..Response::default()
            },
        )]));

        let url = format!("http://127.0.0.1:{}/x", fixture.service.port());
        let response = ureq::get(&url).call().unwrap();

        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("application/json"),
            "a client that refuses a body without its Content-Type is not doing anything unusual, \
             and a fake it rejects tests nothing"
        );
    }

    #[test]
    fn an_unmatched_request_hits_the_catch_all_and_says_so_in_the_journal() {
        let fixture = serving(scenario(vec![
            rule(
                Match {
                    bin: Some("/known".to_string()),
                    ..Match::default()
                },
                Response::default(),
            ),
            catch_all("unforeseen"),
        ]));

        assert_eq!(get(&fixture, "/surprise").0, 500);

        let calls = Journal::read(fixture.journal.path()).unwrap();
        assert_eq!(calls.len(), 1);
        assert!(
            calls[0].catch_all,
            "the catch-all is what turns an unforeseen dependency into a failure instead of \
             silence. It must work identically at both doors, or that guarantee is only half true"
        );
        assert_eq!(calls[0].bin, "/surprise");
    }

    #[test]
    fn the_journal_records_a_request_the_way_it_records_a_call() {
        let fixture = serving(scenario(vec![rule(
            Match::default(),
            Response {
                stdout: Some("ok".to_string()),
                ..Response::default()
            },
        )]));
        get(&fixture, "/orders?page=2");

        let calls = Journal::read(fixture.journal.path()).unwrap();
        assert_eq!(calls[0].bin, "/orders");
        assert!(
            calls[0].args.iter().any(|arg| arg == "GET"),
            "the method belongs in the arguments so `args_contain: POST` works at this door the \
             same way it works for a binary's flags: {:?}",
            calls[0].args
        );
        assert!(
            calls[0].args.iter().any(|arg| arg.contains("page=2")),
            "and so does the query, or a rule could not tell two calls to the same path apart: \
             {:?}",
            calls[0].args
        );
    }

    #[test]
    fn the_call_counter_advances_per_path() {
        let fixture = serving(scenario(vec![
            rule(
                Match {
                    bin: Some("/orders".to_string()),
                    call: Some(2),
                    ..Match::default()
                },
                Response {
                    stdout: Some("second".to_string()),
                    ..Response::default()
                },
            ),
            catch_all("first"),
        ]));

        assert_eq!(get(&fixture, "/orders").1, "first");
        assert_eq!(
            get(&fixture, "/orders").1,
            "second",
            "`call: 2` counts requests to that path, which is what lets a case fake a retry \
             answering differently the second time"
        );
    }

    #[test]
    fn a_rule_can_match_on_the_request_body() {
        let fixture = serving(scenario(vec![
            rule(
                Match {
                    stdin_contains: Some("\"urgent\":true".to_string()),
                    ..Match::default()
                },
                Response {
                    stdout: Some("expedited".to_string()),
                    ..Response::default()
                },
            ),
            catch_all("ordinary"),
        ]));

        let url = format!("http://127.0.0.1:{}/orders", fixture.service.port());
        let mut response = ureq::post(&url).send("{\"urgent\":true}").unwrap();

        assert_eq!(
            response.body_mut().read_to_string().unwrap(),
            "expedited",
            "`stdin_contains` is how a rule tells two calls apart by their payload. Without the \
             body reaching the matcher, faking a POST API means every request looks identical"
        );
    }

    #[test]
    fn a_scenario_written_for_the_binary_door_works_at_this_one() {
        let yaml = "rules:\n  - match: { bin: /health }\n    stdout: \"ok\"\n  - match: {}\n    \
                    stdout: \"unforeseen\"\n    exit: 1\n";
        let loaded: Scenario = serde_yaml_ng::from_str(yaml).unwrap();
        let fixture = serving(loaded);

        assert_eq!(
            get(&fixture, "/health"),
            (200, "ok".to_string()),
            "a project writes `fake.rules` once. A scenario with no `status` anywhere — the shape \
             every existing case already has — must work at this door too, or the two doors are \
             two formats wearing one name"
        );
        assert_eq!(get(&fixture, "/elsewhere").0, 200);
    }

    #[test]
    fn a_render_hook_is_refused_with_a_message_naming_the_limit() {
        let root = tempfile::tempdir().unwrap();
        let refused = FakeService::start(
            Scenario {
                render: Some("./shape.sh".to_string()),
                rules: vec![catch_all("x")],
            },
            root.path().join("journal.jsonl"),
            root.path().to_path_buf(),
            0,
        );

        let message = match refused {
            Ok(_) => panic!("a mode this door cannot honour must not appear to work"),
            Err(error) => error.to_string(),
        };
        assert!(
            message.contains("render"),
            "a scenario asking for something this door does not do must say which mode, not fail \
             obscurely at the first request: {message}"
        );
    }

    #[test]
    fn a_passthrough_rule_is_refused_because_there_is_no_service_further_along() {
        let root = tempfile::tempdir().unwrap();
        let refused = FakeService::start(
            scenario(vec![rule(
                Match::default(),
                Response {
                    exec: Some("real".to_string()),
                    ..Response::default()
                },
            )]),
            root.path().join("journal.jsonl"),
            root.path().to_path_buf(),
            0,
        );

        assert!(
            refused.is_err(),
            "`exec: real` finds the next binary along PATH. There is no next service along a \
             port, so honouring it here would mean inventing a meaning it does not have"
        );
    }
}
