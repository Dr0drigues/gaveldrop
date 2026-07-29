//! The human-readable report as one self-contained page.
//!
//! **Written from scratch, not carried over from the prototype.** Most of the prototype's HTML
//! renders things specific to one project — its event vocabulary, its sqlite counts — and only
//! the shell would have transposed. A shell is quicker to write than to adapt.
//!
//! Self-contained is the requirement that shapes it: no external stylesheet, no script, no remote
//! font. A report is read from a CI artefact, often with no network, and a blank page is worse
//! than a plain one.

use std::io::Write;

use crate::report::{Report, Sink};
use crate::{Diff, Outcome};

/// Writes the page once the suite has finished.
pub struct Html<W: Write> {
    out: W,
}

impl<W: Write> Html<W> {
    /// A renderer writing to `out`.
    pub fn new(out: W) -> Self {
        Self { out }
    }
}

impl<W: Write> Sink for Html<W> {
    fn case_finished(&mut self, _outcome: &Outcome) {}

    fn finish(&mut self, report: &Report) {
        let _ = self.out.write_all(render(report).as_bytes());
        let _ = self.out.flush();
    }
}

/// Renders the whole report as one HTML page.
pub fn render(report: &Report) -> String {
    let summary = report.summary();
    let rows: String = ordered(report).into_iter().map(row).collect();

    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>gaveldrop report</title>\n<style>{STYLE}</style>\n</head>\n<body>\n\
         <h1>gaveldrop</h1>\n<p class=\"summary\">{total} cases · {passed} passed · \
         {failed} failed · {tolerated} tolerated · score {score}/{max}</p>\n\
         <table>{rows}</table>\n</body>\n</html>\n",
        total = summary.total,
        passed = summary.passed,
        failed = summary.failed,
        tolerated = summary.tolerated,
        score = summary.score,
        max = summary.max_score,
    )
}

/// The outcomes with what needs attention first.
///
/// A reader opening a CI artefact should not scroll past ninety green rows to find the one that
/// broke. Real failures, then tolerated ones, then passes; within each group, heavier weights
/// first.
fn ordered(report: &Report) -> Vec<&Outcome> {
    let mut ordered: Vec<&Outcome> = report.outcomes.iter().collect();
    ordered.sort_by_key(|outcome| (rank(outcome), std::cmp::Reverse(outcome.weight)));
    ordered
}

/// How urgent one outcome is: lower sorts earlier.
fn rank(outcome: &Outcome) -> u8 {
    match (outcome.passed, outcome.allow_fail) {
        (false, false) => 0,
        (false, true) => 1,
        (true, _) => 2,
    }
}

/// One table row, with its failures nested underneath.
fn row(outcome: &Outcome) -> String {
    let (class, mark) = match (outcome.passed, outcome.allow_fail) {
        (true, _) => ("ok", "ok"),
        (false, true) => ("warn", "warn"),
        (false, false) => ("fail", "FAIL"),
    };
    let scored = if outcome.passed { outcome.weight } else { 0 };

    let mut detail: String = outcome.diffs.iter().map(diff_row).collect();

    if !outcome.unexpected_calls.is_empty() {
        detail.push_str(&format!(
            "<div class=\"diff\"><code>unexpected calls</code><span>{}</span></div>",
            escape(&outcome.unexpected_calls.join(", "))
        ));
    }
    if !outcome.unmentioned_files.is_empty() {
        detail.push_str(&format!(
            "<div class=\"aside\">also written, not asserted: {}</div>",
            escape(&outcome.unmentioned_files.join(", "))
        ));
    }

    format!(
        "<tr class=\"{class}\"><td class=\"mark\">{mark}</td><td>{name}<div>{detail}</div></td>\
         <td class=\"score\">{scored}/{weight}</td></tr>",
        name = escape(&outcome.name),
        weight = outcome.weight,
    )
}

/// One failed assertion.
fn diff_row(diff: &Diff) -> String {
    format!(
        "<div class=\"diff\"><code>{path}</code><span>expected {expected}</span>\
         <span>got {got}</span></div>",
        path = escape(&diff.path),
        expected = escape(&diff.expected),
        got = escape(&diff.got),
    )
}

/// Escapes text that came from the subject under test.
///
/// A diff carries whatever the tested program printed. Rendering it raw would let a subject inject
/// markup into the report — and the subject is, by definition, the thing whose behaviour is not
/// yet trusted.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The whole stylesheet, inline.
const STYLE: &str = "\
body{font:14px/1.5 ui-monospace,monospace;margin:2rem auto;max-width:60rem;padding:0 1rem}\
h1{font-size:1.2rem;margin:0 0 .25rem}\
.summary{color:#555;margin:0 0 1.5rem}\
table{border-collapse:collapse;width:100%}\
td{border-top:1px solid #e5e5e5;padding:.5rem .4rem;vertical-align:top}\
.mark{width:3.5rem;font-weight:700}\
.score{width:4rem;text-align:right;color:#555}\
tr.ok .mark{color:#1a7f37}tr.warn .mark{color:#9a6700}tr.fail .mark{color:#cf222e}\
.diff{margin:.4rem 0 0 .5rem;border-left:2px solid #e5e5e5;padding-left:.6rem}\
.diff code{display:block;color:#0969da}\
.diff span{display:block;color:#555;white-space:pre-wrap;word-break:break-word}\
.aside{margin:.4rem 0 0 .5rem;color:#777;font-style:italic}\
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Report;
    use crate::{Diff, Outcome};

    fn outcome(name: &str, weight: u32, passed: bool) -> Outcome {
        Outcome {
            name: name.to_string(),
            weight,
            allow_fail: false,
            passed,
            diffs: Vec::new(),
            unexpected_calls: Vec::new(),
            unmentioned_files: Vec::new(),
        }
    }

    #[test]
    fn the_page_is_self_contained() {
        let page = render(&Report::from(vec![outcome("a", 5, true)]));

        for forbidden in ["http://", "https://", "//cdn", "<script src", "<link "] {
            assert!(
                !page.contains(forbidden),
                "a report is read from a CI artefact with no network: {forbidden:?} would leave a \
                 blank page. Found it in:\n{page}"
            );
        }
        assert!(page.contains("<style>"), "styling must be inline");
    }

    #[test]
    fn the_summary_and_every_case_appear() {
        let page = render(&Report::from(vec![
            outcome("passing-case", 5, true),
            outcome("failing-case", 8, false),
        ]));

        for fragment in ["passing-case", "failing-case", "5/13"] {
            assert!(page.contains(fragment), "missing {fragment:?} in:\n{page}");
        }
    }

    #[test]
    fn a_failure_shows_the_expectation_path_and_both_values() {
        let failing = Outcome {
            diffs: vec![Diff {
                path: "expect.stdout.absent[0]".to_string(),
                expected: "nowhere: \"ZSH_ENV\"".to_string(),
                got: "scriptPath: $ZSH_ENV_DIR/x.zsh".to_string(),
            }],
            ..outcome("k9s", 8, false)
        };
        let page = render(&Report::from(vec![failing]));

        for fragment in ["expect.stdout.absent[0]", "ZSH_ENV_DIR"] {
            assert!(page.contains(fragment), "missing {fragment:?}");
        }
    }

    #[test]
    fn values_from_the_subject_are_escaped() {
        let hostile = Outcome {
            diffs: vec![Diff {
                path: "expect.stdout.contains[0]".to_string(),
                expected: "anything".to_string(),
                got: "<script>alert('x')</script> & \"quoted\"".to_string(),
            }],
            ..outcome("hostile", 1, false)
        };
        let page = render(&Report::from(vec![hostile]));

        assert!(
            !page.contains("<script>alert"),
            "a diff carries whatever the subject printed, so rendering it raw would let a tested \
             program inject markup into the report. Got:\n{page}"
        );
        assert!(page.contains("&lt;script&gt;"));
        assert!(page.contains("&amp;"));
    }

    /// Where a case's **row** sits in the page.
    ///
    /// Searching for the bare name would find it in the summary line instead, which sits above the
    /// table and never moves — an ordering assertion built on that would measure nothing.
    fn row_at(page: &str, name: &str) -> usize {
        page.find(&format!("{name}<div>"))
            .unwrap_or_else(|| panic!("no row for {name:?} in:\n{page}"))
    }

    #[test]
    fn a_failing_case_is_rendered_before_a_passing_one() {
        let page = render(&Report::from(vec![
            outcome("passing", 5, true),
            outcome("failing", 8, false),
        ]));

        assert!(
            row_at(&page, "failing") < row_at(&page, "passing"),
            "what needs attention goes first: a reader opening an artefact should not scroll past \
             ninety green rows"
        );
    }

    #[test]
    fn a_tolerated_failure_sorts_between_a_real_one_and_a_pass() {
        let tolerated = Outcome {
            allow_fail: true,
            ..outcome("tolerated", 3, false)
        };
        let page = render(&Report::from(vec![
            outcome("passing", 5, true),
            tolerated,
            outcome("failing", 8, false),
        ]));

        assert!(
            row_at(&page, "failing") < row_at(&page, "tolerated")
                && row_at(&page, "tolerated") < row_at(&page, "passing"),
            "a declared failure is neither urgent nor invisible"
        );
    }

    #[test]
    fn an_empty_report_renders_without_dividing_by_anything() {
        assert!(render(&Report::from(Vec::new())).contains("0/0"));
    }
}
