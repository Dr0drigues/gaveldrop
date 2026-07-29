//! The checks themselves.
//!
//! Each one is built the same way: a fixed case, a real isolation, one invocation through the
//! adapter under test, and a single question about what came back. Every failure carries what was
//! observed, so a third party never has to reproduce the check by hand to see what happened.

use std::collections::BTreeMap;
use std::path::Path;

use gaveldrop::adapters::Adapter;
use gaveldrop::{Case, Expect, Isolation, Observations, Setup};

use crate::{Check, Finding};

/// Runs every check, in a fixed order.
pub fn all(adapter: &dyn Adapter, fake_binary: &Path) -> Vec<Finding> {
    vec![
        exit_code_is_reported(adapter, fake_binary),
        both_streams_are_reported(adapter, fake_binary),
        the_home_directory_is_the_isolated_one(adapter, fake_binary),
        a_cleared_variable_does_not_reach_the_subject(adapter, fake_binary),
        files_written_are_reported(adapter, fake_binary),
        an_unexpected_call_reaches_the_catch_all(adapter, fake_binary),
    ]
}

/// An adapter must report what the subject exited with.
fn exit_code_is_reported(adapter: &dyn Adapter, fake: &Path) -> Finding {
    const CHECK: Check = Check {
        name: "exit_code_is_reported",
        why: "every expectation on exit_code rests on this; an adapter that always reports zero \
              would make every case pass",
    };

    finding(
        CHECK,
        observe(adapter, fake, &["sh", "-c", "exit 7"], &[], &[]),
        |seen| (seen.exit == 7, format!("exit was {}", seen.exit)),
    )
}

/// An adapter must keep the two streams apart.
fn both_streams_are_reported(adapter: &dyn Adapter, fake: &Path) -> Finding {
    const CHECK: Check = Check {
        name: "both_streams_are_reported",
        why: "stdout and stderr are separate assertions; an adapter that merges them would make an \
              `absent` expectation on one silently read the other",
    };

    let run = ["sh", "-c", "echo out; echo err >&2"];
    finding(CHECK, observe(adapter, fake, &run, &[], &[]), |seen| {
        let held = seen.stdout.contains("out")
            && seen.stderr.contains("err")
            && !seen.stdout.contains("err");
        (
            held,
            format!(
                "stdout {:?}, stderr {:?}",
                seen.stdout.trim(),
                seen.stderr.trim()
            ),
        )
    })
}

/// The check that carries the whole edifice.
fn the_home_directory_is_the_isolated_one(adapter: &dyn Adapter, fake: &Path) -> Finding {
    const CHECK: Check = Check {
        name: "the_home_directory_is_the_isolated_one",
        why: "this is the load-bearing invariant: an adapter that lets the subject see the real \
              home makes the suite corrupt the configuration of whoever runs it",
    };

    let case = case(&["sh", "-c", "printf %s \"$HOME\""]);
    let iso = match Isolation::prepare(&case, fake, &[], &[]) {
        Ok(iso) => iso,
        Err(error) => return refused(CHECK, &error.to_string()),
    };
    let expected = iso.root().to_string_lossy().into_owned();

    finding(
        CHECK,
        adapter
            .invoke(&case, &iso)
            .map_err(|error| error.to_string()),
        |seen| {
            let saw = seen.stdout.trim().to_string();
            (saw == expected, format!("the subject saw HOME={saw:?}"))
        },
    )
}

/// A variable listed for clearing must really be gone.
fn a_cleared_variable_does_not_reach_the_subject(adapter: &dyn Adapter, fake: &Path) -> Finding {
    const CHECK: Check = Check {
        name: "a_cleared_variable_does_not_reach_the_subject",
        why: "a project reading its own config variable before HOME would escape isolation \
              entirely if that variable survived",
    };

    let run = ["sh", "-c", "printf %s \"${CONFORMANCE_PROBE-absent}\""];
    let cleared = ["CONFORMANCE_PROBE".to_string()];

    finding(CHECK, observe(adapter, fake, &run, &[], &cleared), |seen| {
        let saw = seen.stdout.trim().to_string();
        (saw == "absent", format!("the subject saw {saw:?}"))
    })
}

/// The tree diff must reach the observations.
fn files_written_are_reported(adapter: &dyn Adapter, fake: &Path) -> Finding {
    const CHECK: Check = Check {
        name: "files_written_are_reported",
        why: "the `files` family exists because some bugs appear in no output at all; an adapter \
              that reports no file effects makes those assertions vacuous",
    };

    let case = case(&["sh", "-c", "printf hello > written.txt"]);
    let mut iso = match Isolation::prepare(&case, fake, &[], &[]) {
        Ok(iso) => iso,
        Err(error) => return refused(CHECK, &error.to_string()),
    };
    iso.snapshot();

    finding(
        CHECK,
        adapter
            .invoke(&case, &iso)
            .map_err(|error| error.to_string()),
        |seen| {
            let held = seen
                .files
                .iter()
                .any(|effect| effect.path.ends_with("written.txt"));
            (held, format!("{} file effects reported", seen.files.len()))
        },
    )
}

/// An unforeseen call must be loud rather than silent.
fn an_unexpected_call_reaches_the_catch_all(adapter: &dyn Adapter, fake: &Path) -> Finding {
    const CHECK: Check = Check {
        name: "an_unexpected_call_reaches_the_catch_all",
        why: "the catch-all is what turns an unforeseen call into a failure instead of silence; an \
              adapter that does not put the fake first on PATH makes every case call the real tool",
    };

    let run = ["sh", "-c", "conformance-probe-tool || true"];
    let bins = ["conformance-probe-tool".to_string()];

    finding(CHECK, observe(adapter, fake, &run, &bins, &[]), |seen| {
        let held = seen.calls.iter().any(|call| call.catch_all);
        (held, format!("{} calls journaled", seen.calls.len()))
    })
}

/// Turns an observation, or the failure to get one, into a finding.
///
/// Factored out so no check can accidentally report `held: true` on an adapter that errored — the
/// one mistake that would let the kit certify something broken.
fn finding(
    check: Check,
    observed: Result<Observations, String>,
    verdict: impl FnOnce(&Observations) -> (bool, String),
) -> Finding {
    match observed {
        Err(detail) => refused(check, &detail),
        Ok(seen) => {
            let (held, detail) = verdict(&seen);
            Finding {
                check,
                held,
                detail,
            }
        }
    }
}

/// A finding for a check that could not even run.
fn refused(check: Check, detail: &str) -> Finding {
    Finding {
        check,
        held: false,
        detail: format!("the adapter could not run the case: {detail}"),
    }
}

/// A minimal case invoking `argv`.
///
/// Built as a value rather than parsed from YAML: the kit's own fixtures are fixed, so a round trip
/// through the loader would only add a way for them to fail.
fn case(argv: &[&str]) -> Case {
    Case {
        name: "conformance".to_string(),
        weight: 1,
        allow_fail: false,
        setup: Setup {
            run: Some(argv.iter().map(|arg| (*arg).to_string()).collect()),
            exec: None,
            extra: BTreeMap::new(),
        },
        fake: None,
        expect: Expect::default(),
    }
}

/// Prepares isolation and invokes, flattening every failure into a message.
fn observe(
    adapter: &dyn Adapter,
    fake: &Path,
    argv: &[&str],
    bins: &[String],
    cleared: &[String],
) -> Result<Observations, String> {
    let case = case(argv);
    let iso = Isolation::prepare(&case, fake, bins, cleared).map_err(|error| error.to_string())?;
    adapter
        .invoke(&case, &iso)
        .map_err(|error| error.to_string())
}
