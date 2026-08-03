//! The human-readable report.

use std::io::Write;

use anstyle::{AnsiColor, Style};

use crate::Outcome;
use crate::report::{Report, Sink, duration, failure_lines};

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
            "{} {}  {}/{}{}",
            self.paint(style, mark),
            outcome.name,
            scored,
            outcome.weight,
            noted(outcome.duration_ms)
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
            "\ngaveldrop — {} {} · {} passed · {} failed · {} tolerated · score {}/{} · {}",
            summary.total,
            plural(summary.total),
            summary.passed,
            summary.failed,
            summary.tolerated,
            summary.score,
            summary.max_score,
            duration(summary.duration_ms)
        );
        if let Some(line) = slowest(report) {
            let _ = writeln!(self.out, "{line}");
        }
        let _ = self.out.flush();
    }
}

/// The cases worth looking at, named once the run is over.
///
/// **This is what a duration is actually for**, and a per-case column cannot provide it. A renderer
/// that streams knows nothing about the distribution while it prints case three of ninety, so the
/// only place a ranking can exist is here. Measured on this repository's own suite: ten cases from
/// 17ms to 521ms, and every one of them under the threshold that would have printed a number — the
/// one case thirty times slower than the rest was invisible until this line existed.
///
/// Three, because a ranking is for deciding where to look and nobody investigates ten things.
fn slowest(report: &Report) -> Option<String> {
    // Under three cases there is nothing to rank, and the summary's total already says everything
    // there is to say about one or two.
    const ENOUGH_TO_RANK: usize = 3;
    // And below this, no case is worth anyone's afternoon. Naming the three slowest of a suite where
    // everything finishes in 20ms is a line that costs attention and buys nothing.
    const WORTH_CHASING: u64 = 100;
    const NAMED: usize = 3;

    if report.outcomes.len() < ENOUGH_TO_RANK {
        return None;
    }

    let mut ranked: Vec<&Outcome> = report.outcomes.iter().collect();
    ranked.sort_by_key(|outcome| std::cmp::Reverse(outcome.duration_ms));

    if ranked.first()?.duration_ms < WORTH_CHASING {
        return None;
    }

    let named: Vec<String> = ranked
        .iter()
        .take(NAMED)
        .map(|outcome| format!("{} {}", outcome.name, duration(outcome.duration_ms)))
        .collect();

    Some(format!("slowest — {}", named.join(" · ")))
}

/// A case's duration, but only once it is worth a reader's attention.
///
/// **Quiet by default, on purpose.** Forty lines each ending in `2ms` is forty columns of noise
/// hiding the one that says `4.1s`, and the point of putting a number on a terminal line is that it
/// stands out. Nothing is lost by the silence: every case's exact duration is in the JSON Lines
/// report and in the HTML table, which is where you compare one run against another anyway — the
/// terminal cannot answer that question no matter how much it prints.
fn noted(ms: u64) -> String {
    // A second. Below it, a case is indistinguishable from every other by eye; above it, someone is
    // waiting. The threshold is not tunable because a knob here would only ever be turned once, and
    // the two sinks that always carry the number make it unnecessary.
    const WORTH_SAYING: u64 = 1_000;

    if ms >= WORTH_SAYING {
        format!("  {}", duration(ms))
    } else {
        String::new()
    }
}

/// `case` or `cases`, so a single-case run does not read as a typo.
///
/// Small, and worth its own function: `--only` and `--shard` make one-case runs ordinary rather than
/// a curiosity, and a summary saying `1 cases` undermines the care taken with every other line.
fn plural(total: usize) -> &'static str {
    if total == 1 { "case" } else { "cases" }
}
