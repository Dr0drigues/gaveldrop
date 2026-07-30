//! What the engine decided, printed before each case runs.
//!
//! A separate renderer rather than a flag inside the terminal one, for the reason every renderer
//! here is separate: it is the only way `--verbose` composes with `--report-json` and the rest
//! without any of them knowing about it.
//!
//! It prints before the verdict on purpose. A case that hangs, or that takes the subject down with
//! it, still leaves behind what it was about to do — which is exactly when a reader needs it and
//! exactly when a report printed afterwards never arrives.

use std::io::Write;

use crate::report::{Report, Sink};
use crate::verdict::Outcome;

/// Writes the engine's own decisions to a stream, one block per case.
pub struct Verbose<W: Write> {
    out: W,
}

impl<W: Write> Verbose<W> {
    /// A renderer writing to `out`.
    pub fn new(out: W) -> Self {
        Self { out }
    }
}

impl<W: Write> Sink for Verbose<W> {
    fn preparing(&mut self, case: &str, note: &[String]) {
        let _ = writeln!(self.out, "···  {case}");
        for line in note {
            let _ = writeln!(self.out, "       {line}");
        }
    }

    /// Nothing. The verdict is the terminal renderer's job, and printing it twice would make a
    /// verbose run harder to read than a quiet one.
    fn case_finished(&mut self, _outcome: &Outcome) {}

    /// Nothing, for the same reason.
    fn finish(&mut self, _report: &Report) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(case: &str, note: &[&str]) -> String {
        let mut out = Vec::new();
        let owned: Vec<String> = note.iter().map(|line| (*line).to_string()).collect();
        Verbose::new(&mut out).preparing(case, &owned);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn the_case_is_named_and_every_line_is_indented_under_it() {
        let text = rendered(
            "deploys-cleanly",
            &["adapter    process", "root       /tmp/x"],
        );

        assert!(
            text.starts_with("···  deploys-cleanly\n"),
            "the case has to be named, or a suite of ninety cases produces ninety anonymous \
             blocks: {text:?}"
        );
        assert!(
            text.contains("\n       adapter    process\n"),
            "and each line sits under it, so the block reads as belonging to that case: {text:?}"
        );
    }

    #[test]
    fn a_verdict_is_not_printed_here() {
        let mut out = Vec::new();
        {
            let mut verbose = Verbose::new(&mut out);
            verbose.case_finished(&Outcome {
                name: "t".to_string(),
                weight: 1,
                allow_fail: false,
                passed: true,
                diffs: Vec::new(),
                unexpected_calls: Vec::new(),
                unmentioned_files: Vec::new(),
            });
            verbose.finish(&Report::from(Vec::new()));
        }

        assert!(
            out.is_empty(),
            "the terminal renderer already prints verdicts, and printing them twice would make a \
             verbose run harder to read than a quiet one — which would defeat the option"
        );
    }
}
