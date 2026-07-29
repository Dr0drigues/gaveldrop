//! The kit run against the shell adapter.
//!
//! The checks are about the isolation contract, not about how a subject is invoked. An adapter that
//! cannot be handed a `run:` command line must still be checkable, or the kit only ever certifies
//! adapters shaped like the first one — and "shaped like the first one" is exactly the deformation
//! the kit exists to prevent.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "panicking is how a test reports failure; an integration test file is its own crate"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use assert_cmd::cargo::cargo_bin;
use gaveldrop::adapters::Adapter;
use gaveldrop::{Case, Expect, Process, Setup, Shell};
use serde_json::json;

/// Turns a check's script into a case the shell adapter claims.
///
/// `eval` rather than a function definition: a check's script is one snippet of shell, and wrapping
/// it in a function would add a scope that changes what `$HOME` and a redirection do.
fn as_shell(script: &str) -> Case {
    Case {
        name: "conformance".to_string(),
        weight: 1,
        allow_fail: false,
        setup: Setup {
            run: None,
            exec: None,
            extra: BTreeMap::from([
                ("shell".to_string(), json!("bash")),
                ("source".to_string(), json!([])),
                ("call".to_string(), json!(["eval", script])),
            ]),
        },
        fake: None,
        expect: Expect::default(),
    }
}

fn fake_binary() -> PathBuf {
    let path = cargo_bin("gaveldrop-fake");
    assert!(
        path.is_file(),
        "{} is missing. Run `cargo test --workspace` rather than testing this crate alone: the \
         fake binary belongs to another crate and is not built by this one's dependency graph.",
        path.display()
    );
    path
}

#[test]
fn the_factory_really_produces_a_shell_case() {
    let built = as_shell("true");

    assert!(
        Shell.claims(&built),
        "without this the battery could be green because the cases quietly went through some \
         other adapter, and `the_shell_adapter_is_conformant` would be measuring nothing"
    );
    assert!(
        !Process.claims(&built),
        "and no other adapter may claim it, or which one ran depends on registry order"
    );
}

#[test]
fn the_shell_adapter_is_conformant() {
    let report = gaveldrop_conformance::run_with(&Shell, &fake_binary(), &as_shell);

    assert!(
        report.is_conformant(),
        "the shell adapter honours the same isolation contract as the process one. If it does not, \
         `Observations` is not the boundary it claims to be, and an expectation written once would \
         mean different things in different technologies:\n{}",
        report.render()
    );
}

#[test]
fn the_process_adapter_is_still_conformant_through_the_default_factory() {
    let report = gaveldrop_conformance::run(&Process, &fake_binary());

    assert!(
        report.is_conformant(),
        "adding a factory must not change what the kit asks of the adapter it already \
         certified:\n{}",
        report.render()
    );
}

#[test]
fn the_two_adapters_are_checked_on_the_same_battery() {
    let shell = gaveldrop_conformance::run_with(&Shell, &fake_binary(), &as_shell);
    let process = gaveldrop_conformance::run(&Process, &fake_binary());

    let names = |report: &gaveldrop_conformance::ConformanceReport| -> Vec<&'static str> {
        report
            .findings
            .iter()
            .map(|finding| finding.check.name)
            .collect()
    };

    assert_eq!(
        names(&shell),
        names(&process),
        "a factory must change how the subject is invoked and nothing else. If one adapter is \
         asked fewer questions than the other, the kit is no longer one contract"
    );
}
