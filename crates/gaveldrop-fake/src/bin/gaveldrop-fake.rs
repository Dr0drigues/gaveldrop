//! The fake binary.
//!
//! Symlinked under the name of every dependency to fake and placed first on `PATH`. It
//! discovers who it stands in for through its `argv[0]`, picks a rule, responds, and
//! journals — in that order, and the journal is never skipped.
//!
//! Five environment variables link it to the core, all documented in
//! [`gaveldrop_fake::env`]. It takes no arguments of its own: it receives exactly those
//! of the binary it replaces.

use std::process::ExitCode;

use gaveldrop_fake::{Call, Counter, Invocation, Journal, RenderPayload, Scenario};

/// Exit code reserved for the fake's own failures. Distinct from anything a rule could
/// simulate — otherwise a broken scenario would pass for a tool that failed, and the
/// case would lie.
const HARNESS_FAILURE: u8 = 125;

fn main() -> ExitCode {
    match run() {
        Ok(code) => exit_code_from(code),
        Err(message) => {
            eprintln!("gaveldrop-fake: {message}");
            ExitCode::from(HARNESS_FAILURE)
        }
    }
}

/// Turns a process exit code into an [`ExitCode`].
///
/// A code outside a byte's range collapses to 1: that is what the shell would make of
/// it anyway, so being explicit here costs nothing.
fn exit_code_from(code: i32) -> ExitCode {
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

/// The body, split out of `main` so that every failure surfaces through `Err` and takes
/// the same exit code.
///
/// Three decisions happen here, in this order:
///
/// 1. Standard input is read only when a rule needs it. Reading it unconditionally
///    would block forever whenever the caller handed over an inherited pipe it never
///    closes.
/// 2. The counter key is the name of the faked binary. A project wanting different
///    semantics — an agent identifier, say — builds its own binary and passes whatever
///    key it likes to [`Counter::next`].
/// 3. Journaling comes **after** responding, so the line carries the real exit code. It
///    is unconditional: catch-all, passthrough, simulated failure — everything lands in
///    the journal, or the counts drawn from it would mean nothing.
fn run() -> Result<i32, String> {
    let scenario = Scenario::from_env().map_err(|error| format!("{error}"))?;

    let inv = Invocation::from_env(scenario.needs_stdin());

    let key = inv.bin.clone();
    let call = Counter::from_env()
        .and_then(|counter| counter.next(&key))
        .map_err(|error| format!("{error}"))?;

    let rule = scenario.select(&inv, call).ok_or_else(|| {
        "no rule matched even though the scenario has a catch-all — engine bug".to_string()
    })?;

    let catch_all = rule.matcher.is_catch_all();
    let passthrough = rule.response.is_passthrough();

    let payload = RenderPayload {
        rule,
        invocation: &inv,
        call,
    };
    let exit = gaveldrop_fake::apply(&rule.response, &inv, scenario.render.as_deref(), &payload)
        .map_err(|error| format!("{error}"))?;

    Journal::from_env()
        .and_then(|journal| {
            journal.record(&Call::from_invocation(
                &inv,
                call,
                &key,
                catch_all,
                passthrough,
                exit,
            ))
        })
        .map_err(|error| format!("{error}"))?;

    Ok(exit)
}
