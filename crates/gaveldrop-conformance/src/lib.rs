//! The battery every adapter must pass to prove it honours the contract.
//!
//! This kit is **gaveldrop's own guarantee**. That a particular consumer passes its own tests is
//! not one: those cases belong to the consumer, they can change without notice, and copying them
//! here would make them diverge at the first change.
//!
//! It has a second use, and it is the less obvious one: it stops the core from deforming when a
//! technology is added, and it gives a third party the means to validate their own adapter without
//! reading our code.

use std::path::Path;

use gaveldrop::adapters::Adapter;

pub mod checks;

/// One thing an adapter must do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Check {
    /// Short identifier, stable enough to grep for.
    pub name: &'static str,
    /// What it protects, in a sentence a third party can act on.
    pub why: &'static str,
}

/// What one check found.
#[derive(Debug, Clone)]
pub struct Finding {
    /// The check that ran.
    pub check: Check,
    /// Whether it held.
    pub held: bool,
    /// What was observed, whether it held or not.
    pub detail: String,
}

/// Everything the kit found.
#[derive(Debug, Clone, Default)]
pub struct ConformanceReport {
    /// One finding per check, in the order they ran.
    pub findings: Vec<Finding>,
}

impl ConformanceReport {
    /// Whether every check held.
    pub fn is_conformant(&self) -> bool {
        self.findings.iter().all(|finding| finding.held)
    }

    /// The findings as readable lines, failures first, each with the reason it exists.
    ///
    /// The reason is printed rather than the name alone: a third party fixing their adapter needs
    /// to know what the check protects, and sending them to our source to find out would defeat
    /// the point of shipping a kit.
    pub fn render(&self) -> String {
        let mut lines: Vec<String> = Vec::new();

        for finding in self.findings.iter().filter(|finding| !finding.held) {
            lines.push(format!(
                "FAIL {}\n     why  {}\n     got  {}",
                finding.check.name, finding.check.why, finding.detail
            ));
        }
        for finding in self.findings.iter().filter(|finding| finding.held) {
            lines.push(format!("ok   {}", finding.check.name));
        }

        lines.join("\n")
    }
}

/// Runs the whole battery against `adapter`.
///
/// `fake_binary` is the fake executable the checks symlink into place. Passed in rather than
/// located, because a third party may be testing against a fake they built themselves from
/// `gaveldrop-fake` as a library — which is how a project supplies its own response rendering.
pub fn run(adapter: &dyn Adapter, fake_binary: &Path) -> ConformanceReport {
    ConformanceReport {
        findings: checks::all(adapter, fake_binary),
    }
}
