//! Adapters: invoke the subject, return normalised observations.

pub mod process;

use crate::{Case, Isolation, Observations};

pub use process::Process;

/// Invokes the subject and returns what it produced.
///
/// An adapter invokes and observes. It **never evaluates** — no adapter knows what a case
/// expects. That is what guarantees an expectation written once behaves identically
/// whatever the technology.
pub trait Adapter {
    /// Runs `case` inside `iso` and reports what happened.
    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError>;
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
}
