//! A test engine where one case is one YAML file.
//!
//! The core knows no language, no framework, no tool. It knows processes, files and
//! lines of text: it loads cases, prepares isolation, asks an adapter to invoke, then
//! evaluates expectations against normalised observations.
//!
//! Everything that knows about a particular technology lives in an adapter or in an
//! executable supplied by the project.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "panicking is how a test reports failure"
    )
)]

pub mod adapters;
pub mod case;
pub mod config;
pub mod hooks;
pub mod iso;
pub mod observations;
pub mod report;
pub mod runner;
pub mod verdict;

pub use adapters::{Adapter, AdapterError, Process};
pub use case::schema;
pub use case::{Case, CaseError, Expect, Setup, TextExpectation};
pub use config::{Config, ConfigError, FakeConfig};
/// The fake's types that appear in this crate's own public API.
///
/// [`Call`] is what [`Observations::calls`] holds, [`Scenario`] what [`Case::fake`] holds, and
/// [`JournalError`] a variant of [`AdapterError`]. Without these, someone writing an adapter
/// outside this crate cannot name the types their own code returns, and would have to depend on
/// `gaveldrop-fake` directly with no way to know which version matches.
pub use gaveldrop_fake::{Call, Journal, JournalError, Match, Response, Rule, Scenario};
pub use hooks::{HookError, run_setup};
pub use iso::snapshot::{FileChange, FileEffect, Snapshot};
pub use iso::{IsoError, Isolation};
pub use observations::Observations;
pub use report::merge::MergeError;
pub use report::{Report, Sink, Summary, Tee};
pub use verdict::{Diff, Outcome, evaluate};
