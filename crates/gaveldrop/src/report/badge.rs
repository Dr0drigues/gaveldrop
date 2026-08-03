//! A badge carrying the verdict, written at the end of a run.
//!
//! `docs/badge.svg` says a suite exists. This one says what it found, which is a different claim and
//! a heavier one: it is true of **the run that wrote the file** and of nothing else. Whoever looks at
//! it in a README is looking at a photograph, not a live reading, and the `<title>` says so — a badge
//! implying otherwise would be the kind of green this project exists to refuse.
//!
//! No service involved. The file is written where the project asks and it is the project's business
//! whether that gets committed, published to Pages or thrown away; gaveldrop stays a program you run.

use std::io::Write;

use crate::report::{Report, Sink};
use crate::verdict::Outcome;

/// Writes an SVG badge once the run is over.
pub struct Badge<W: Write> {
    out: W,
}

impl<W: Write> Badge<W> {
    /// A badge renderer writing to `out`.
    pub fn new(out: W) -> Self {
        Self { out }
    }
}

impl<W: Write> Sink for Badge<W> {
    /// Nothing per case: a badge is one value and it needs the totals.
    fn case_finished(&mut self, _outcome: &Outcome) {}

    fn finish(&mut self, report: &Report) {
        let summary = report.summary();
        let value = format!("{}/{}", summary.score, summary.max_score);
        let _ = write!(
            self.out,
            "{}",
            render(&value, colour(report), &title(report))
        );
    }
}

/// The verdict as a colour, which is what a badge is read for at a glance.
///
/// Three states rather than two. A tolerated failure must not look like a clean run — that is what
/// declaring `allow_fail` asked for — nor like a broken one, or the exemption becomes a way to hide
/// things instead of to declare them.
fn colour(report: &Report) -> &'static str {
    let summary = report.summary();
    if !report.is_success() {
        "#b3403f"
    } else if summary.tolerated > 0 {
        "#9a6b12"
    } else {
        "#2e6f52"
    }
}

/// What the badge cannot show, said where a reader can find it.
///
/// The counts belong here rather than in the badge: a label wide enough for "10 cases, 2 tolerated"
/// stops being a badge. The score is the number the project stands behind — cases are not worth the
/// same — and the rest is one hover away.
fn title(report: &Report) -> String {
    let summary = report.summary();
    format!(
        "gaveldrop: {}/{} weighted, {} of {} cases passed, {} tolerated — as of the run that wrote \
         this file",
        summary.score, summary.max_score, summary.passed, summary.total, summary.tolerated
    )
}

/// The badge itself.
///
/// Widths are computed from the text rather than fixed, because `1234/5678` is as legitimate a value
/// as `5/5` and a fixed box would either clip it or leave a hole. The 7-pixel figure is the width of
/// a digit in the 11-pixel Verdana this asks for; the fallbacks are wider, so the box has room in the
/// case that is not measured.
fn render(value: &str, colour: &str, title: &str) -> String {
    const LABEL: &str = "gaveldrop";
    let label_width = 12 + LABEL.len() * 7;
    let value_width = 12 + value.chars().count() * 7;
    let total = label_width + value_width;

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total}" height="20" role="img" aria-label="gaveldrop {value}">
  <title>{title}</title>
  <rect width="{label_width}" height="20" fill="#4a4f5a"/>
  <rect x="{label_width}" width="{value_width}" height="20" fill="{colour}"/>
  <g font-family="Verdana,Geneva,DejaVu Sans,sans-serif" font-size="11" fill="#ffffff">
    <text x="{label_middle}" y="14" text-anchor="middle">{LABEL}</text>
    <text x="{value_middle}" y="14" text-anchor="middle" font-weight="bold">{value}</text>
  </g>
</svg>
"##,
        label_middle = label_width / 2,
        value_middle = label_width + value_width / 2,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::Diff;

    fn outcome(name: &str, weight: u32, passed: bool, allow_fail: bool) -> Outcome {
        Outcome {
            name: name.to_string(),
            weight,
            allow_fail,
            passed,
            diffs: if passed {
                Vec::new()
            } else {
                vec![Diff {
                    path: "expect.exit_code".to_string(),
                    expected: "0".to_string(),
                    got: "1".to_string(),
                }]
            },
            unexpected_calls: Vec::new(),
            unmentioned_files: Vec::new(),
            duration_ms: 0,
        }
    }

    fn badge_for(outcomes: Vec<Outcome>) -> String {
        let report = Report::from(outcomes);
        let mut out = Vec::new();
        {
            let mut badge = Badge::new(&mut out);
            badge.finish(&report);
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn a_clean_run_shows_the_full_score_in_green() {
        let svg = badge_for(vec![
            outcome("a", 5, true, false),
            outcome("b", 3, true, false),
        ]);

        assert!(
            svg.contains(">8/8<"),
            "the weighted score, not a count: {svg}"
        );
        assert!(
            svg.contains("#2e6f52"),
            "the colour is what a badge is read for at a glance: {svg}"
        );
    }

    #[test]
    fn a_failure_is_red_and_the_score_says_how_much_held() {
        let svg = badge_for(vec![
            outcome("a", 5, true, false),
            outcome("b", 3, false, false),
        ]);

        assert!(svg.contains(">5/8<"), "{svg}");
        assert!(svg.contains("#b3403f"), "{svg}");
    }

    #[test]
    fn a_tolerated_failure_looks_like_neither_of_the_other_two() {
        let svg = badge_for(vec![
            outcome("a", 5, true, false),
            outcome("known", 3, false, true),
        ]);

        assert!(
            svg.contains("#9a6b12"),
            "a tolerated failure must not look like a clean run — that is what declaring \
             `allow_fail` asked for — nor like a broken one, or the exemption becomes a way to hide \
             things: {svg}"
        );
    }

    #[test]
    fn the_title_says_the_badge_is_a_photograph() {
        let svg = badge_for(vec![outcome("a", 5, true, false)]);

        assert!(
            svg.contains("as of the run that wrote this file"),
            "a badge showing a score is true of one run and of nothing else. Implying a live \
             reading would be the kind of green this project exists to refuse: {svg}"
        );
        assert!(
            svg.contains("1 of 1 cases passed"),
            "the counts go in the title, since a label wide enough for them stops being a badge: \
             {svg}"
        );
    }

    #[test]
    fn a_wide_value_gets_a_wider_box() {
        let narrow = badge_for(vec![outcome("a", 5, true, false)]);
        let wide = badge_for(vec![outcome("a", 4_000, true, false)]);

        let width = |svg: &str| -> usize {
            svg.split("width=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .and_then(|number| number.parse().ok())
                .unwrap()
        };

        assert!(
            width(&wide) > width(&narrow),
            "`4000/4000` is as legitimate a value as `5/5`, and a fixed box would clip it or leave \
             a hole: {} against {}",
            width(&wide),
            width(&narrow)
        );
    }

    #[test]
    fn an_empty_report_does_not_divide_by_anything() {
        let svg = badge_for(Vec::new());

        assert!(
            svg.contains(">0/0<"),
            "a suite that discovered nothing is a loud error elsewhere; here it must simply not \
             panic: {svg}"
        );
    }
}
