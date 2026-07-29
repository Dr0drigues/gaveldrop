//! The kit run against the web adapter.
//!
//! A third calling convention, the same six checks. The point is that the checks are about the
//! isolation contract and not about how a subject is invoked — an adapter whose subject stays alive
//! and answers requests must honour exactly what one that runs to completion honours.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "panicking is how a test reports failure; an integration test file is its own crate"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use assert_cmd::cargo::cargo_bin;
use gaveldrop::adapters::Adapter;
use gaveldrop::{Case, Expect, Process, Setup, Shell, Web};
use serde_json::json;

/// Turns a check's script into a case the web adapter claims.
///
/// No `steps:` and no `ready:`, which is what makes this work: with no exchange to perform there is
/// nothing to be ready *for*, so the adapter waits for the subject to finish instead of waiting for a
/// port to open. The checks assert on what the script produced — its exit code, its streams, the
/// files it wrote — and none of that needs a listener.
fn as_web(script: &str) -> Case {
    Case {
        name: "conformance".to_string(),
        weight: 1,
        allow_fail: false,
        setup: Setup {
            run: None,
            exec: None,
            extra: BTreeMap::from([("serve".to_string(), json!(["sh", "-c", script]))]),
        },
        fake: None,
        expect: Expect::default(),
        steps: Vec::new(),
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
fn the_factory_really_produces_a_web_case() {
    let built = as_web("true");

    assert!(
        Web.claims(&built),
        "without this the battery could be green because the cases quietly went through another \
         adapter, and the conformance test below would be measuring nothing"
    );
    assert!(
        !Shell.claims(&built) && !Process.claims(&built),
        "and no other adapter may claim it, or which one ran depends on registry order rather \
         than on what the case declares"
    );
}

#[test]
fn the_web_adapter_is_conformant() {
    let report = gaveldrop_conformance::run_with(&Web, &fake_binary(), &as_web);

    assert!(
        report.is_conformant(),
        "a subject that stays alive is still subject to the same isolation contract. If it is not, \
         an expectation written once means different things depending on the technology, which is \
         the one thing this project cannot allow:\n{}",
        report.render()
    );
}

#[test]
fn all_three_adapters_are_asked_the_same_questions() {
    let names = |report: &gaveldrop_conformance::ConformanceReport| -> Vec<&'static str> {
        report
            .findings
            .iter()
            .map(|finding| finding.check.name)
            .collect()
    };

    let web = gaveldrop_conformance::run_with(&Web, &fake_binary(), &as_web);
    let process = gaveldrop_conformance::run(&Process, &fake_binary());

    assert_eq!(
        names(&web),
        names(&process),
        "a factory changes how the subject is invoked and nothing else. An adapter asked fewer \
         questions than another would be certified against a weaker contract"
    );
}
