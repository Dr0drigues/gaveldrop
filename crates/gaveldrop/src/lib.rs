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

pub mod case;
pub mod iso;

pub use case::schema;
pub use case::{Case, CaseError, Expect, Setup, TextExpectation};
pub use iso::{IsoError, Isolation};
