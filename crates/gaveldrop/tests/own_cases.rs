//! gaveldrop's own cases, run through gaveldrop.
//!
//! The smallest honest form of dogfooding available at this stage: the case files under
//! `tests/cases/` are real, and this test is what keeps them working. The day the tool
//! breaks, these cases say so before CI does.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "panicking is how a test reports failure; an integration test file is its own \
              crate, so the library's cfg_attr does not cover it"
)]

use std::path::PathBuf;

use assert_cmd::cargo::cargo_bin;
use gaveldrop::report::terminal::Terminal;
use gaveldrop::{Config, runner};

/// The repository root, resolved from this crate's manifest rather than the working
/// directory, so the test behaves the same however cargo was invoked.
fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The fake binary, with an actionable message when it has not been built.
///
/// `CARGO_BIN_EXE_*` is only defined for integration tests of the crate that **declares**
/// the binary, and `gaveldrop-fake` is declared elsewhere. So the path is computed instead,
/// which means it can be missing: `cargo test -p gaveldrop` builds dependencies as
/// libraries, while `cargo test --workspace` builds every binary.
fn fake_binary() -> PathBuf {
    let path = cargo_bin("gaveldrop-fake");
    assert!(
        path.is_file(),
        "{} is missing. Run `cargo test --workspace` (or `mise run test`) rather than \
         testing this crate alone: the fake binary belongs to another crate and is not \
         built by this one's dependency graph.",
        path.display()
    );
    path
}

#[test]
fn every_case_in_the_repository_passes() {
    let root = repository_root();
    let config = Config::load(&root.join("gaveldrop.yaml")).unwrap();

    let mut rendered = Vec::new();
    let report = {
        let mut sink = Terminal::plain(&mut rendered);
        runner::run_all(&config, &root, &fake_binary(), &mut sink).unwrap()
    };

    assert!(
        report.is_success(),
        "gaveldrop's own cases must pass, or the tool does not work:\n{}",
        String::from_utf8_lossy(&rendered)
    );
    assert!(
        report.summary().total > 0,
        "a suite with no cases would pass while proving nothing"
    );
}

#[test]
fn the_repository_config_declares_the_binaries_its_cases_fake() {
    let config = Config::load(&repository_root().join("gaveldrop.yaml")).unwrap();

    assert!(
        config.fake.bins.contains(&"git".to_string()),
        "a case faking `git` needs `git` listed in `fake.bins`, or no symlink is laid down \
         and the real tool answers instead. Declared: {:?}",
        config.fake.bins
    );
}
