//! The kit run against the one adapter that exists.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "panicking is how a test reports failure; an integration test file is its own crate"
)]

use std::path::PathBuf;

use assert_cmd::cargo::cargo_bin;
use gaveldrop::Process;

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
fn the_process_adapter_is_conformant() {
    let report = gaveldrop_conformance::run(&Process, &fake_binary());

    assert!(
        report.is_conformant(),
        "the reference adapter must pass its own kit, or the kit is measuring something else:\n{}",
        report.render()
    );
}

#[test]
fn every_check_actually_ran() {
    let report = gaveldrop_conformance::run(&Process, &fake_binary());

    assert!(
        report.findings.len() >= 6,
        "a kit that quietly skips checks would certify anything. Ran: {}",
        report.findings.len()
    );
}

#[test]
fn each_check_explains_why_it_exists() {
    for finding in gaveldrop_conformance::run(&Process, &fake_binary()).findings {
        assert!(
            finding.check.why.len() > 30,
            "a third party reading a failure needs to know what the check protects, not just its \
             name. `{}` says: {:?}",
            finding.check.name,
            finding.check.why
        );
    }
}

#[test]
fn no_two_checks_share_a_name() {
    let report = gaveldrop_conformance::run(&Process, &fake_binary());
    let mut names: Vec<&str> = report
        .findings
        .iter()
        .map(|finding| finding.check.name)
        .collect();
    names.sort_unstable();
    let before = names.len();
    names.dedup();

    assert_eq!(
        names.len(),
        before,
        "a name is what a third party greps for in a failure, so two checks sharing one would \
         send them to the wrong place"
    );
}
