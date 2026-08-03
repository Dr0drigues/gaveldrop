//! The human-readable report as one self-contained page.
//!
//! **Written from scratch, not carried over from the prototype.** Most of the prototype's HTML
//! renders things specific to one project — its event vocabulary, its sqlite counts — and only
//! the shell would have transposed. A shell is quicker to write than to adapt.
//!
//! Self-contained is the requirement that shapes it: no external stylesheet, no script, no remote
//! font. A report is read from a CI artefact, often with no network, and a blank page is worse
//! than a plain one.

use std::collections::BTreeMap;
use std::io::Write;

use crate::report::{Report, Sink};
use crate::{Diff, Observations, Outcome};

/// Writes the page once the suite has finished.
pub struct Html<W: Write> {
    out: W,
    seen: BTreeMap<String, Observations>,
}

impl<W: Write> Html<W> {
    /// A renderer writing to `out`.
    pub fn new(out: W) -> Self {
        Self {
            out,
            seen: BTreeMap::new(),
        }
    }
}

impl<W: Write> Sink for Html<W> {
    fn case_finished(&mut self, _outcome: &Outcome) {}

    /// Kept until `finish`, because a page is written once and in one piece.
    ///
    /// Held by case name, which is what the outcome carries too. A duplicate name would overwrite,
    /// and that is the right failure: two cases sharing a name are indistinguishable in every report
    /// this project produces, so the answer is to rename one.
    fn observed(&mut self, case: &str, observations: &Observations) {
        self.seen.insert(case.to_string(), observations.clone());
    }

    fn finish(&mut self, report: &Report) {
        let _ = self
            .out
            .write_all(render_with(report, &self.seen).as_bytes());
        let _ = self.out.flush();
    }
}

/// Renders the whole report as one HTML page.
///
/// Kept for callers that have no observations to hand; `render_with` is what the sink uses.
pub fn render(report: &Report) -> String {
    render_with(report, &BTreeMap::new())
}

/// Renders the page, with each case's own run foldable underneath it.
pub fn render_with(report: &Report, seen: &BTreeMap<String, Observations>) -> String {
    let summary = report.summary();
    let rows: String = ordered(report)
        .into_iter()
        .map(|outcome| row(outcome, seen.get(&outcome.name)))
        .collect();

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

/// One table row, with its failures nested underneath and its run one click away.
fn row(outcome: &Outcome, observed: Option<&Observations>) -> String {
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

    // Appended after the diffs, never around them. A failure has to be readable without a click:
    // folding the verdict away to make the page tidier would trade the one thing a report is for.
    if let Some(observations) = observed {
        detail.push_str(&run_detail(observations));
    }

    format!(
        "<tr class=\"{class}\"><td class=\"mark\">{mark}</td><td>{name}<div>{detail}</div></td>\
         <td class=\"score\">{scored}/{weight}</td></tr>",
        name = escape(&outcome.name),
        weight = outcome.weight,
    )
}

/// What the subject actually did, folded shut.
///
/// `<details>` rather than a script: this page has no JavaScript and that is a property worth
/// keeping — it is read from a CI artefact, sometimes with no network, and every browser has
/// implemented folding natively for years.
///
/// Nothing is rendered when there is nothing to show. An empty fold invites a click that answers
/// no question, which is worse than no fold at all.
fn run_detail(observations: &Observations) -> String {
    let mut parts = vec![field("exit", &observations.exit.to_string())];

    if !observations.stdout.is_empty() {
        parts.push(stream("stdout", &observations.stdout));
    }
    if !observations.stderr.is_empty() {
        parts.push(stream("stderr", &observations.stderr));
    }
    if !observations.calls.is_empty() {
        parts.push(field("calls", &counted(observations)));
    }
    if !observations.files.is_empty() {
        let written: Vec<String> = observations
            .files
            .iter()
            .map(|effect| format!("{} ({} bytes)", effect.path.display(), effect.size))
            .collect();
        parts.push(field("files", &written.join(", ")));
    }
    if let Some(status) = observations.status {
        parts.push(field("status", &status.to_string()));
    }

    format!(
        "<details><summary>what it did</summary><div class=\"obs\">{}</div></details>",
        parts.join("")
    )
}

/// One labelled value.
fn field(label: &str, value: &str) -> String {
    format!(
        "<code>{label}</code><span>{}</span>",
        escape(&capped(value))
    )
}

/// One stream, kept as written so its line breaks survive.
fn stream(label: &str, text: &str) -> String {
    format!("<code>{label}</code><pre>{}</pre>", escape(&capped(text)))
}

/// Which binaries were called, and how often.
fn counted(observations: &Observations) -> String {
    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    for call in &observations.calls {
        *tally.entry(call.bin.as_str()).or_default() += 1;
    }
    tally
        .into_iter()
        .map(|(bin, count)| format!("{bin} ×{count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// As much of a value as belongs in a page, and no more.
///
/// A subject that writes a hundred thousand lines would otherwise produce a report nobody can open.
/// The cut is generous — a page has room a terminal line does not — and it names what it left out,
/// so a truncation is never mistaken for the whole output.
fn capped(text: &str) -> String {
    const ROOM: usize = 4_000;

    match text.char_indices().nth(ROOM) {
        None => text.to_string(),
        Some((cut, _)) => format!("{}\n… ({} bytes in all)", &text[..cut], text.len()),
    }
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
.aside{margin:.4rem 0 0 .5rem;color:#777;font-style:italic}details{margin:.4rem 0 0 .5rem}summary{cursor:pointer;color:#555;font-size:.9em}summary:hover{color:#0969da}.obs{margin:.3rem 0 0 .6rem;border-left:2px solid #e5e5e5;padding-left:.6rem}.obs code{display:block;color:#0969da;margin-top:.3rem}.obs span{display:block;color:#333}.obs pre{margin:.1rem 0 0;white-space:pre-wrap;word-break:break-word;color:#333;font:inherit}\
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

    fn observations(stdout: &str) -> Observations {
        Observations {
            exit: 0,
            stdout: stdout.to_string(),
            ..Default::default()
        }
    }

    fn page_with(outcome: Outcome, observed: Observations) -> String {
        let name = outcome.name.clone();
        render_with(
            &Report::from(vec![outcome]),
            &BTreeMap::from([(name, observed)]),
        )
    }

    #[test]
    fn a_passing_case_can_be_unfolded_to_see_what_it_did() {
        let page = page_with(outcome("quiet", 5, true), observations("ready\n"));

        assert!(
            page.contains("<details>") && page.contains("what it did"),
            "the point of the fold is a case that passed: its verdict says nothing about what the \
             subject actually wrote, called or created:\n{page}"
        );
        assert!(
            page.contains("ready"),
            "and what it wrote has to be in there:\n{page}"
        );
    }

    #[test]
    fn no_script_is_involved() {
        let page = page_with(outcome("a", 5, true), observations("x"));

        assert!(
            !page.contains("<script") && !page.contains("onclick"),
            "`<details>` is native and every browser has folded with it for years. A report is read \
             from a CI artefact, sometimes with no network, and this page has never needed \
             JavaScript:\n{page}"
        );
    }

    #[test]
    fn a_failure_is_readable_without_unfolding_anything() {
        let mut broken = outcome("broken", 8, false);
        broken.diffs = vec![Diff {
            path: "expect.exit_code".to_string(),
            expected: "0".to_string(),
            got: "1".to_string(),
        }];
        let page = page_with(broken, observations("some output"));

        let verdict = page.find("expect.exit_code").unwrap();
        let fold = page.find("<details>").unwrap();

        assert!(
            verdict < fold,
            "the diffs come before the fold and outside it. Folding a verdict away to tidy the page \
             would trade the one thing a report exists for:\n{page}"
        );
    }

    #[test]
    fn a_case_with_no_observations_gets_no_empty_fold() {
        let page = render(&Report::from(vec![outcome("never-ran", 5, false)]));

        assert!(
            !page.contains("<details>"),
            "a case that never ran — a broken document, an adapter that claimed nothing — has \
             nothing to show. A fold inviting a click that answers no question is worse than \
             none:\n{page}"
        );
    }

    #[test]
    fn a_subjects_markup_cannot_escape_the_fold_either() {
        let page = page_with(
            outcome("a", 5, true),
            observations("<script>alert('x')</script>"),
        );

        assert!(
            !page.contains("<script>alert"),
            "a stream carries whatever the tested program printed, and the subject is by \
             definition the thing not yet trusted:\n{page}"
        );
        assert!(
            page.contains("&lt;script&gt;"),
            "escaped, not dropped:\n{page}"
        );
    }

    #[test]
    fn a_huge_stream_is_capped_and_says_by_how_much() {
        let flood = "x".repeat(50_000);
        let page = page_with(outcome("noisy", 5, true), observations(&flood));

        assert!(
            page.len() < 20_000,
            "a subject writing fifty thousand characters must not produce a page nobody can open: \
             {} bytes",
            page.len()
        );
        assert!(
            page.contains("50000 bytes in all"),
            "and the reader is told what was left out, so a cut is never mistaken for the whole \
             output:\n{}",
            &page[..600]
        );
    }

    #[test]
    fn calls_are_tallied_rather_than_listed_one_by_one() {
        let mut observed = observations("");
        observed.calls = vec![
            gaveldrop_fake::Call {
                bin: "git".into(),
                args: vec!["status".into()],
                call: 1,
                key: "git".into(),
                catch_all: false,
                passthrough: false,
                exit: 0,
            },
            gaveldrop_fake::Call {
                bin: "git".into(),
                args: vec!["log".into()],
                call: 2,
                key: "git".into(),
                catch_all: false,
                passthrough: false,
                exit: 0,
            },
        ];

        let page = page_with(outcome("a", 5, true), observed);

        assert!(
            page.contains("git ×2"),
            "a case calling one tool forty times should read as a count, not forty rows:\n{page}"
        );
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
