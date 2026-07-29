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
    fn claims(&self, case: &Case) -> bool {
        case.setup.extra.contains_key("serve")
    }

    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError> {
        let argv = serve_command(case)?;
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
    let probe = case
        .setup
        .extra
        .get("ready")
        .and_then(|value| value.as_str())
        .map(str::to_string);

    subject
        .wait_until_ready(probe.as_deref(), READY_TIMEOUT)
        .map_err(|error| AdapterError::Unsupported {
            case: case.name.clone(),
            reason: error.to_string(),
        })?;

    let agent = agent();
    let port = port_of(iso, "GAVELDROP_PORT");

    Ok((
        0,
        case.steps
            .iter()
            .map(|step| request::perform(&agent, &request::read(&step.request), port))
            .collect(),
    ))
}

/// The command line that starts the service.
fn serve_command(case: &Case) -> Result<Vec<String>, AdapterError> {
    case.setup
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
        })
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
