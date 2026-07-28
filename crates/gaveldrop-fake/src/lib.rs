//! Rule engine for faked dependencies.
//!
//! This crate has two faces. As a **binary**, it is symlinked under the name of
//! every dependency to fake and placed first on `PATH`: it matches the call,
//! responds, and journals it. As a **library**, it lets a project build its own
//! fake binary with its own response rendering.
//!
//! It depends on no other crate in this repository, and that must stay true: a
//! consumer that only wants the engine must not have to pull in the case format,
//! the evaluation and the reports.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "panicking is how a test reports failure"
    )
)]

pub mod env;
pub mod invocation;

pub use invocation::Invocation;
