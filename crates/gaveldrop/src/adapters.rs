//! Adapters: invoke the subject, return normalised observations.

pub mod process;
pub mod shell;
pub mod web;

use std::process::{Command, Stdio};

use crate::{Case, Isolation, Observations};

pub use process::Process;
pub use shell::Shell;
pub use web::Web;

/// Invokes the subject and returns what it produced.
///
/// An adapter invokes and observes. It **never evaluates** — no adapter knows what a case
/// expects. That is what guarantees an expectation written once behaves identically
/// whatever the technology.
pub trait Adapter {
    /// Whether this adapter is the one for `case`.
    ///
    /// Declaring a capability, not judging a result: the invariant above is about expectations, and
    /// this reads none. Selection lives in the case rather than in configuration so that a project
    /// mixing a binary and the shell scripts around it does not have to split its suite.
    fn claims(&self, case: &Case) -> bool;

    /// Runs `case` inside `iso` and reports what happened.
    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError>;

    /// How this adapter calls itself, for the trace `--verbose` prints.
    ///
    /// Defaulted rather than required, unlike `claims`. The reasoning is the opposite of the one
    /// there: a wrong default for `claims` fails quietly, whereas a nameless adapter only makes one
    /// line of a diagnostic vaguer. Requiring it would break every consumer adapter already written
    /// against this trait — which right now means armadai's, mid-migration — for a label.
    fn name(&self) -> &str {
        "a consumer-provided adapter"
    }
}

/// Every adapter, in the order they are asked.
///
/// Order matters at exactly one point: a case could plausibly name both `serve:` and `shell:`, since
/// starting a service written as a shell function is not absurd. The more specific claim has to win,
/// so `Web` is asked first.
pub fn registry() -> Vec<Box<dyn Adapter>> {
    vec![Box::new(Web), Box::new(Shell), Box::new(Process)]
}

/// Runs `command` to completion, feeding it `stdin` when the case declared one.
///
/// Shared rather than written in each adapter, because the pipe has a trap in it. Writing the input
/// on this thread and *then* reading the output deadlocks as soon as the subject fills its own
/// output pipe before it has finished reading its input — a filter over more than a pipe's worth of
/// data does exactly that. So the input goes out on its own thread while `wait_with_output` drains
/// the other two.
///
/// A write that fails is ignored on purpose: a subject is entitled to stop reading — `head -1` does
/// — and closing the pipe early is its business, not an error in the case.
pub fn invoke(command: &mut Command, stdin: Option<&str>) -> std::io::Result<std::process::Output> {
    let Some(input) = stdin else {
        return command.output();
    };

    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn()?;
    let payload = input.to_string();

    let writer = child.stdin.take().map(|mut pipe| {
        std::thread::spawn(move || {
            use std::io::Write;
            let _ = pipe.write_all(payload.as_bytes());
        })
    });

    let output = child.wait_with_output()?;
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    Ok(output)
}

/// The adapter for `case`.
///
/// Each is asked in turn rather than trying `invoke` until one succeeds: trying would run the
/// subject, and a case must never execute against an adapter that was not meant for it.
pub fn select<'a>(
    case: &Case,
    from: &'a [Box<dyn Adapter>],
) -> Result<&'a dyn Adapter, AdapterError> {
    from.iter()
        .find(|adapter| adapter.claims(case))
        .map(AsRef::as_ref)
        .ok_or_else(|| AdapterError::Unclaimed {
            case: case.name.clone(),
            keys: case.setup.extra.keys().cloned().collect(),
        })
}

/// What can go wrong while invoking a subject.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    /// The case does not give this adapter enough to work with.
    #[error("case `{case}` cannot be invoked by this adapter: {reason}")]
    Unsupported {
        /// The case's name.
        case: String,
        /// What is missing.
        reason: String,
    },
    /// The subject could not be started.
    #[error("starting `{program}`: {source}")]
    Spawn {
        /// The program that would not start.
        program: String,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The call journal could not be read back.
    #[error("reading the call journal: {0}")]
    Journal(#[from] gaveldrop_fake::JournalError),
    /// No adapter recognised the case, so nothing would be invoked.
    ///
    /// This is the refusal that used to live in `Case::load`, and it guards the same thing: a case
    /// that parses and then invokes nothing is a green test asserting about a program that never
    /// started. It moved here because `run` and `exec` stopped being the criterion the moment there
    /// was more than one adapter.
    ///
    /// The listed keys are the rest of the diagnostic. Because `Setup::extra` accepts any key by
    /// design, a mistyped one cannot be rejected by the format — so the reader is shown what was
    /// actually there and spots `shel` against `shell` themselves.
    #[error(
        "case `{case}` would invoke nothing: no adapter recognises it. Add `run: [...]` with a \
         command line, `shell:` with `call:` for a shell function, or `serve:` for a service. \
         `setup.exec` only prepares \
         the directory, so it is not enough on its own. setup holds {}",
        if keys.is_empty() { "no other key".to_string() } else { keys.join(", ") }
    )]
    Unclaimed {
        /// The case's name.
        case: String,
        /// The keys `setup` did hold.
        keys: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn case(yaml: &str) -> Case {
        Case::load_str(yaml, Path::new("inline")).unwrap()
    }

    const WITH_RUN: &str =
        "name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nexpect: { exit_code: 0 }\n";
    const WITH_SHELL: &str =
        "name: t\nweight: 1\nsetup:\n  shell: bash\n  call: [\"f\"]\nexpect: { exit_code: 0 }\n";

    #[test]
    fn a_case_with_run_goes_to_an_adapter_that_takes_a_command_line() {
        let adapters = registry();
        let chosen = select(&case(WITH_RUN), &adapters).unwrap();

        assert!(chosen.claims(&case(WITH_RUN)));
        assert!(
            !chosen.claims(&case(WITH_SHELL)),
            "the two adapters must not both claim everything, or the registry order decides by \
             accident rather than the case deciding"
        );
    }

    #[test]
    fn a_case_with_shell_goes_to_the_shell_adapter() {
        let adapters = registry();
        let chosen = select(&case(WITH_SHELL), &adapters).unwrap();

        assert!(chosen.claims(&case(WITH_SHELL)));
        assert!(!chosen.claims(&case(WITH_RUN)));
    }

    fn refusal(yaml: &str) -> String {
        let adapters = registry();
        match select(&case(yaml), &adapters) {
            Ok(_) => panic!("some adapter claimed a case that names neither `run` nor `shell`"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn a_mistyped_key_is_told_which_keys_were_seen() {
        let message = refusal(
            "name: t\nweight: 1\nsetup:\n  shel: bash\n  call: [\"f\"]\nexpect: { exit_code: 0 }\n",
        );

        for expected in ["shel", "call"] {
            assert!(
                message.contains(expected),
                "`extra` accepts any key by design, so a typo cannot be rejected by the format. \
                 Listing what was seen is what lets a reader spot it without opening our source. \
                 Missing {expected:?} in: {message}"
            );
        }
    }

    #[test]
    fn a_case_claimed_by_no_one_names_the_case_and_says_what_would_work() {
        let message = refusal("name: lonely\nweight: 1\nsetup: {}\nexpect: { exit_code: 0 }\n");

        assert!(message.contains("lonely"));
        assert!(
            message.contains("run") && message.contains("shell"),
            "naming the keys that would have worked is what turns a dead end into a next step: \
             {message}"
        );
    }
}
