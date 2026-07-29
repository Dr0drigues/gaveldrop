//! The human-readable report.

use std::io::Write;

use anstyle::{AnsiColor, Style};

use crate::Outcome;
use crate::report::{Report, Sink, failure_lines};

/// Writes outcomes to a stream as they finish, then a summary.
pub struct Terminal<W: Write> {
    out: W,
    styled: bool,
}

impl<W: Write> Terminal<W> {
    /// A renderer with no styling, for tests and for pipes.
    pub fn plain(out: W) -> Self {
        Self { out, styled: false }
    }

    /// A renderer that styles its output.
    pub fn styled(out: W) -> Self {
        Self { out, styled: true }
    }

    /// Wraps `text` in `style`, or leaves it alone when styling is off.
    fn paint(&self, style: Style, text: &str) -> String {
        if self.styled {
            format!("{style}{text}{style:#}")
        } else {
            text.to_string()
        }
    }
}

/// How one outcome opens its line: the marker, and the colour that goes with it.
///
/// A tolerated failure must not look like a passing case, nor like a real failure —
/// otherwise `allow_fail` becomes a way to hide things rather than to declare them.
fn marker_for(outcome: &Outcome) -> (&'static str, Style) {
    if outcome.passed {
        ("ok  ", Style::new().fg_color(Some(AnsiColor::Green.into())))
    } else if outcome.allow_fail {
        (
            "warn",
            Style::new().fg_color(Some(AnsiColor::Yellow.into())),
        )
    } else {
        ("FAIL", Style::new().fg_color(Some(AnsiColor::Red.into())))
    }
}

impl<W: Write> Sink for Terminal<W> {
    fn case_finished(&mut self, outcome: &Outcome) {
        let (mark, style) = marker_for(outcome);
        let scored = if outcome.passed { outcome.weight } else { 0 };

        let _ = writeln!(
            self.out,
            "{} {}  {}/{}",
            self.paint(style, mark),
            outcome.name,
            scored,
            outcome.weight
        );
        for line in failure_lines(outcome) {
            let _ = writeln!(self.out, "{line}");
        }
        let _ = self.out.flush();
    }

    fn finish(&mut self, report: &Report) {
        let summary = report.summary();
        let _ = writeln!(
            self.out,
            "\ngaveldrop — {} {} · {} passed · {} failed · {} tolerated · score {}/{}",
            summary.total,
            plural(summary.total),
            summary.passed,
            summary.failed,
            summary.tolerated,
            summary.score,
            summary.max_score
        );
        let _ = self.out.flush();
    }
}

/// `case` or `cases`, so a single-case run does not read as a typo.
///
/// Small, and worth its own function: `--only` and `--shard` make one-case runs ordinary rather than
/// a curiosity, and a summary saying `1 cases` undermines the care taken with every other line.
fn plural(total: usize) -> &'static str {
    if total == 1 { "case" } else { "cases" }
}
