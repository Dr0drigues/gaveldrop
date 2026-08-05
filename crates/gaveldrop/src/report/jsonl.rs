//! The machine-readable report: one JSON object per line, outcomes only.
//!
//! JSON Lines because the two things this format must do pull in opposite directions otherwise.
//! A single JSON document cannot be appended to as cases finish — it is invalid until its
//! closing brace — and two of them cannot be merged by concatenation.
//!
//! **No summary line is ever written.** The summary is always computed from the outcomes, which
//! is what makes `cat shard-*.jsonl > all.jsonl` exactly the right operation with nothing to
//! filter out. Humans get the computed summary from the terminal renderer.

use std::io::Write;

use crate::Outcome;
use crate::report::{Report, Sink};

/// Writes one JSON object per finished case.
pub struct Jsonl<W: Write> {
    out: W,
}

impl<W: Write> Jsonl<W> {
    /// A renderer writing to `out`.
    pub fn new(out: W) -> Self {
        Self { out }
    }
}

impl<W: Write> Sink for Jsonl<W> {
    fn case_finished(&mut self, outcome: &Outcome) {
        if let Ok(line) = serde_json::to_string(outcome) {
            let _ = writeln!(self.out, "{line}");
            let _ = self.out.flush();
        }
    }

    fn finish(&mut self, _report: &Report) {
        let _ = self.out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Diff;

    fn outcome(name: &str, weight: u32, passed: bool) -> Outcome {
        Outcome {
            name: name.to_string(),
            weight,
            allow_fail: false,
            passed,
            diffs: Vec::new(),
            unexpected_calls: Vec::new(),
            unmentioned_files: Vec::new(),
            duration_ms: 0,
        }
    }

    fn rendered(outcomes: &[Outcome], call_finish: bool) -> String {
        let report = Report::from(outcomes.to_vec());
        let mut buffer = Vec::new();
        {
            let mut sink = Jsonl::new(&mut buffer);
            for outcome in &report.outcomes {
                sink.case_finished(outcome);
            }
            if call_finish {
                sink.finish(&report);
            }
        }
        String::from_utf8_lossy(&buffer).into_owned()
    }

    #[test]
    fn one_line_per_outcome_valid_json_each() {
        let text = rendered(&[outcome("a", 5, true), outcome("b", 3, false)], true);
        let lines: Vec<&str> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();

        assert_eq!(lines.len(), 2, "text was:\n{text}");
        for line in lines {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(value["name"].is_string());
        }
    }

    #[test]
    fn no_summary_line_is_written_ever() {
        let text = rendered(&[outcome("a", 5, true)], true);

        assert!(
            !text.contains("\"total\"") && !text.contains("\"max_score\""),
            "the summary is always computed, never stored: that is what makes two reports \
             mergeable by plain concatenation, with nothing to filter out. Got:\n{text}"
        );
    }

    #[test]
    fn a_line_is_written_before_finish_is_ever_called() {
        let text = rendered(&[outcome("first", 5, true)], false);

        assert!(
            text.contains("first"),
            "outcomes must reach the file as they happen: a live consumer tailing it cannot wait \
             for the suite to end. Got:\n{text}"
        );
    }

    #[test]
    fn a_failing_outcome_carries_its_diffs() {
        let failing = Outcome {
            diffs: vec![Diff {
                path: "expect.exit_code".to_string(),
                expected: "0".to_string(),
                got: "3".to_string(),
                help: None,
            }],
            ..outcome("broken", 8, false)
        };
        let text = rendered(&[failing], true);
        let value: serde_json::Value = serde_json::from_str(text.trim()).unwrap();

        assert_eq!(value["diffs"][0]["path"], "expect.exit_code");
        assert_eq!(value["diffs"][0]["got"], "3");
    }
}
