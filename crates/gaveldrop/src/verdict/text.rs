//! Expectations on a stream of text.

use crate::TextExpectation;
use crate::verdict::Diff;

/// Checks `expectation` against `stream`, prefixing every diff path with `prefix`.
///
/// `prefix` is what makes a failure locatable — `expect.stdout.contains[1]` rather than
/// "a contains expectation failed".
pub fn check(expectation: &TextExpectation, stream: &str, prefix: &str) -> Vec<Diff> {
    let owned;
    let stream = if expectation.ignore_ansi {
        owned = stripped(stream);
        owned.as_str()
    } else {
        stream
    };

    check_against(expectation, stream, prefix)
}

/// The comparisons themselves, on whatever text `check` decided to compare.
fn check_against(expectation: &TextExpectation, stream: &str, prefix: &str) -> Vec<Diff> {
    let mut diffs = Vec::new();

    if let Some(want) = &expectation.equals
        && without_final_newline(stream) != without_final_newline(want)
    {
        let (expected, got) = mismatch(stream, want);
        diffs.push(Diff {
            path: format!("{prefix}.equals"),
            expected,
            got,
        });
    }

    for (index, needle) in expectation.contains.iter().enumerate() {
        if !stream.contains(needle.as_str()) {
            diffs.push(Diff {
                path: format!("{prefix}.contains[{index}]"),
                expected: format!("contains {needle:?}"),
                got: excerpt(stream),
            });
        }
    }

    for (index, group) in expectation.line_includes.iter().enumerate() {
        let at = format!("{prefix}.line_includes[{index}]");

        // A group with nothing in it is satisfied by every line, so it is an assertion that cannot
        // fail — the one thing this project refuses everywhere else. Said here rather than refused at
        // load time because this is where the path and the stream already are.
        if group.is_empty() {
            diffs.push(Diff {
                path: at,
                expected: "at least one value to look for".to_string(),
                got: "an empty list, which every line satisfies. Name the values, or remove the entry"
                    .to_string(),
            });
            continue;
        }

        if lines(stream)
            .iter()
            .any(|line| absent_from(line, group).is_empty())
        {
            continue;
        }

        diffs.push(Diff {
            path: at,
            expected: format!("one line holding all of {group:?}"),
            got: match closest_line(stream, group) {
                Some((number, line, missing)) => {
                    format!("the closest was line {number}  {line}, missing {missing:?}")
                }
                None => format!("no line holds any of them. {}", excerpt(stream)),
            },
        });
    }

    for (index, needle) in expectation.absent.iter().enumerate() {
        if let Some(at) = stream.find(needle.as_str()) {
            diffs.push(Diff {
                path: format!("{prefix}.absent[{index}]"),
                expected: format!("nowhere: {needle:?}"),
                got: around(stream, at),
            });
        }
    }

    diffs
}

/// What to print for a failed `equals`: the two values, or the line where they part company.
///
/// **One line each way is readable; ten is not.** A single-line mismatch — a version banner, an error
/// message — is understood at a glance from the two values, and a report that also announced the
/// column would be adding arithmetic to something already obvious.
///
/// Multi-line is the opposite. `visible` renders newlines as `⏎`, so a ten-line expectation arrives as
/// one long line of glyphs, and the stream beside it is cut at 120 characters. The reader is then
/// comparing two mangled strings by eye to find one wrong character. So above one line the report
/// stops showing the values and shows the divergence: which line, and what each side has there.
fn mismatch(stream: &str, want: &str) -> (String, String) {
    let whitespace = differs_only_in_whitespace(stream, want);

    if lines(stream).len() < 2 && lines(want).len() < 2 {
        return (
            visible(want),
            if whitespace {
                format!("{} — the same but for whitespace", visible(stream))
            } else {
                excerpt(stream)
            },
        );
    }

    let (expected, got) = diverging(stream, want);
    let got = if whitespace {
        format!("{got} — the same but for whitespace")
    } else {
        got
    };
    (expected, got)
}

/// The first line the two texts disagree on, and what each side holds there.
///
/// Line-oriented rather than character-oriented because that is the unit multi-line output is written
/// in and read in: "line 7 differs" sends someone to a place in a file, where "byte 214 differs" sends
/// them to count. Within the line the values are shown in full, so the wrong character is right there
/// beside the right one.
///
/// A text running out is a divergence too, and the commonest one: the expectation says four lines and
/// the subject wrote three. Saying so plainly beats printing an empty value on one side.
fn diverging(stream: &str, want: &str) -> (String, String) {
    let (found, wanted) = (lines(stream), lines(want));

    let at = (0..)
        .find(|index| found.get(*index) != wanted.get(*index))
        .unwrap_or(0);
    let number = at + 1;

    match (wanted.get(at), found.get(at)) {
        // Only this arm says how far the two agreed. In the other two the sentence already carries
        // it — "the stream ends after 3 lines" cannot be true of a divergence at line 4 unless the
        // first three matched — and saying it twice reads as two facts rather than one.
        (Some(expected), Some(got)) => (
            format!("line {number}  {}", shown(expected)),
            format!("line {number}  {}{}", shown(got), matched(at)),
        ),
        // The expectation asks for a line the subject never wrote.
        (Some(expected), None) => (
            format!("line {number}  {}", shown(expected)),
            format!("the stream ends after {} {}", at, plural_lines(at)),
        ),
        // The subject wrote past where the expectation stops.
        (None, Some(got)) => (
            format!("nothing after line {at}"),
            format!(
                "line {number}  {} — {} {} in all",
                shown(got),
                found.len(),
                plural_lines(found.len())
            ),
        ),
        // Unreachable: the two texts are unequal, so some index differs.
        (None, None) => (visible(want), excerpt(stream)),
    }
}

/// How far the two texts agreed, for a divergence deep in a long output.
///
/// The point of it: `line 47 differs` leaves open whether lines 1 to 46 are fine or whether 47 is
/// merely the first of many. Saying they matched closes that question, which is the difference
/// between reading one line and re-reading the whole stream.
fn matched(before: usize) -> String {
    match before {
        0 => String::new(),
        1 => " (line 1 matched)".to_string(),
        _ => format!(" (the first {before} lines matched)"),
    }
}

/// The text's lines, with one trailing newline ignored the same way `equals` ignores it.
///
/// Without this, a stream ending in `\n` has a phantom empty last line and every multi-line
/// comparison would report a divergence one past the end of the expectation.
fn lines(text: &str) -> Vec<&str> {
    let text = without_final_newline(text);
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n').collect()
}

/// Which of `group` are not among `line`'s words.
///
/// **Words rather than substrings, which is the half that makes the assertion bite.** `inactive`
/// contains `active`, so a substring comparison would hold on the very row a case is written to catch —
/// the same trap `args_include` exists for one crate over. Splitting on whitespace also makes the
/// comparison indifferent to how a table is padded, which is the other thing asked of it.
fn absent_from(line: &str, group: &[String]) -> Vec<String> {
    group
        .iter()
        .filter(|want| !line.split_whitespace().any(|word| word == want.as_str()))
        .cloned()
        .collect()
}

/// The line sharing the most of `group`, with what it was missing — or nothing worth pointing at.
///
/// The same reasoning as the closest event: a case whose group failed nearly always has a line holding
/// part of it, and naming that line beside what it lacked is the whole diagnostic. Where no line shares
/// a single value there is no near miss, and the stream itself is the better answer.
///
/// Between two lines equally close, the one holding the group's **first** value wins. In
/// `["DOCKER", "inactive"]` against a table, both the `DOCKER` row and some other row carrying
/// `inactive` miss exactly one value — and the row the reader wants named is the one they asked about.
/// The first value is the row key in every table anyone writes.
fn closest_line(stream: &str, group: &[String]) -> Option<(usize, String, Vec<String>)> {
    let head = group.first();

    lines(stream)
        .iter()
        .enumerate()
        .map(|(index, line)| (index + 1, shown(line), absent_from(line, group)))
        .filter(|(_, _, missing)| missing.len() < group.len())
        .min_by_key(|(_, _, missing)| {
            (
                missing.len(),
                u8::from(head.is_some_and(|first| missing.contains(first))),
            )
        })
}

/// One line of a text, made visible, capped, and quoted so a trailing space has somewhere to be.
///
/// Quoted by hand rather than with `{:?}`: `visible` has already escaped what needed escaping, and
/// debug formatting on top of it turns its `\e` into `\\e` — an escape of an escape, which is
/// exactly the confusion this whole path exists to prevent.
fn shown(line: &str) -> String {
    format!("\"{}\"", capped(&visible(line), 120, line.len()))
}

/// `line` or `lines`, so a one-line stream does not read as a typo.
fn plural_lines(count: usize) -> &'static str {
    if count == 1 { "line" } else { "lines" }
}

/// What the stream held, for a failure message: as much as fits, on one line, readable.
///
/// It used to be the **first line** of the stream. That reads as the whole answer and is not, and it
/// cost the first real consumer of the shell adapter most of a debugging session: their subject
/// started with a colour escape followed by a newline, so `got` was one invisible sequence and the
/// report showed an empty value. An empty `got` on a stream assertion means "the subject wrote
/// nothing", so they went looking for a function that was not running — and it was running fine.
///
/// Two things follow. The whole stream is shown rather than its first line, with newlines made
/// visible so it still occupies one line of the report; and control bytes are escaped, so a stream
/// that is invisible can never be mistaken for a stream that is absent.
fn excerpt(stream: &str) -> String {
    if stream.is_empty() {
        return "(empty)".to_string();
    }

    let shown = visible(stream.trim());
    if shown.is_empty() {
        return format!("({} bytes, all of them whitespace)", stream.len());
    }

    // 120, the same cap as before this changed. Widening it was tempting and pointless: the stream
    // that caused the trouble renders to about forty-five characters, so the cap was never what hid
    // it — taking only the first line was.
    capped(&shown, 120, stream.len())
}

/// The line containing byte offset `at`, so an `absent` failure shows the offender in
/// context rather than the whole stream.
fn around(stream: &str, at: usize) -> String {
    let start = stream[..at].rfind('\n').map_or(0, |index| index + 1);
    let end = stream[at..]
        .find('\n')
        .map_or(stream.len(), |index| at + index);
    let line = stream[start..end].trim();
    capped(&visible(line), 120, line.len())
}

/// The text minus **one** trailing newline, which is what `equals` compares.
///
/// A shell subject ends its output with a newline almost always, and a case never writes one — so a
/// byte-exact comparison would fail every first attempt for a reason nothing in the case explains.
/// One newline, not all trailing whitespace: two blank lines at the end may well be the bug being
/// hunted, and swallowing them would make the assertion weaker than the person writing it believes.
fn without_final_newline(text: &str) -> &str {
    text.strip_suffix('\n').unwrap_or(text)
}

/// True when the two are the same once every whitespace byte is taken out.
///
/// Only used to add a sentence to a failure. Two values differing by a tab, a trailing space or a
/// second newline render identically in a report, and "expected X, got X" sends the reader looking
/// for a bug in the comparison rather than in the whitespace.
fn differs_only_in_whitespace(stream: &str, want: &str) -> bool {
    let squeeze = |text: &str| -> String { text.chars().filter(|c| !c.is_whitespace()).collect() };
    squeeze(stream) == squeeze(want)
}

/// The text with terminal escape sequences taken out.
///
/// Two families, not one. **CSI** — `ESC [` up to a byte in `@`–`~` — is what colours are, and it is
/// what anyone thinks of. **OSC** — `ESC ]` up to a bell or `ESC \` — carries window titles and
/// hyperlinks, and a tool that emits one usually emits both. Handling only the colours would move the
/// problem one step along rather than solve it, and the next reader would have no idea why half the
/// escapes went and half stayed.
///
/// Written by hand rather than with a crate: this is thirty lines against a dependency, in a project
/// with fourteen of them.
fn stripped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(character) = chars.next() {
        if character != '\u{1b}' {
            out.push(character);
            continue;
        }

        match chars.peek() {
            // CSI: parameters and intermediates, then one final byte that ends it.
            Some('[') => {
                chars.next();
                for inside in chars.by_ref() {
                    if ('@'..='~').contains(&inside) {
                        break;
                    }
                }
            }
            // OSC: a string terminated by BEL, or by ESC \ — consume both bytes of the latter.
            Some(']') => {
                chars.next();
                while let Some(inside) = chars.next() {
                    match inside {
                        '\u{7}' => break,
                        '\u{1b}' => {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
            // A lone escape, or a two-byte sequence like ESC c. Dropping the escape and keeping
            // what follows is the least surprising choice: it is text the subject wrote.
            _ => {}
        }
    }

    out
}

/// Control bytes rendered so they can be seen.
///
/// A report is read by a person in a terminal, which *interprets* an escape sequence rather than
/// showing it — so the bytes that most need to be visible in a diagnostic are exactly the ones a
/// terminal hides. Escapes are the common case in shell output: any tool that colours its output
/// produces them, and `contains:` fails on them.
fn visible(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\n' => "⏎ ".to_string(),
            '\t' => "→ ".to_string(),
            '\u{1b}' => "\\e".to_string(),
            other if other.is_control() => format!("\\x{:02x}", other as u32),
            other => other.to_string(),
        })
        .collect()
}

/// Caps a fragment so one long stream cannot drown a report, naming what was left out.
///
/// The original length is the stream's, not the rendered one's: a reader wants to know how much the
/// subject wrote, and `visible` makes that number bigger for reasons of its own.
fn capped(text: &str, limit: usize, original: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let kept: String = text.chars().take(limit).collect();
    format!("{kept}… ({original} bytes in all)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextExpectation;

    fn contains(needle: &str) -> TextExpectation {
        TextExpectation {
            contains: vec![needle.to_string()],
            ..Default::default()
        }
    }

    fn absent(needle: &str) -> TextExpectation {
        TextExpectation {
            absent: vec![needle.to_string()],
            ..Default::default()
        }
    }

    fn equals(want: &str) -> TextExpectation {
        TextExpectation {
            equals: Some(want.to_string()),
            ..Default::default()
        }
    }

    fn on_one_line(values: &[&str]) -> TextExpectation {
        TextExpectation {
            line_includes: vec![values.iter().map(|value| (*value).to_string()).collect()],
            ..Default::default()
        }
    }

    /// The table a consumer inverted, whose four words stayed present while every status was wrong.
    const TABLE: &str = "MODULE       STATUS\nKUBE         inactive\nDOCKER       active\n";

    /// `contains` never says two fragments belong together, and that let an inverted table pass.
    ///
    /// **The injected defect was `if enabled` flipped to `if !enabled`**, which swapped every status in
    /// a `MODULE / STATUS` table. The case asserted `contains: ["KUBE", "DOCKER", "active",
    /// "inactive"]`: all four words were still there, the assertion still held, and the output was
    /// entirely wrong. Reported by the consumer who injected it to measure their suite.
    #[test]
    fn a_line_group_catches_what_contains_cannot() {
        let scattered = TextExpectation {
            contains: vec![
                "KUBE".to_string(),
                "DOCKER".to_string(),
                "active".to_string(),
                "inactive".to_string(),
            ],
            ..Default::default()
        };

        assert!(
            check(&scattered, TABLE, "expect.stdout").is_empty(),
            "the four words are all present in the inverted table, which is exactly the hole"
        );
        assert_eq!(
            check(&on_one_line(&["KUBE", "active"]), TABLE, "expect.stdout").len(),
            1,
            "while asking for the two together fails on it — `KUBE` is on the inactive row. This is \
             also where a substring comparison would have quietly held: `inactive` contains `active`, \
             so words are not a refinement here, they are the assertion"
        );
        assert!(
            check(&on_one_line(&["KUBE", "inactive"]), TABLE, "expect.stdout").is_empty(),
            "and holds on the row that really carries both"
        );
    }

    /// The only answer available before was freezing the spacing, which is what this replaces.
    #[test]
    fn a_line_group_does_not_care_how_the_columns_are_padded() {
        let widened = "MODULE         STATUS\nKUBE           inactive\n";

        assert!(
            check(
                &on_one_line(&["KUBE", "inactive"]),
                widened,
                "expect.stdout"
            )
            .is_empty(),
            "the alternative was `contains: [\"KUBE         inactive\"]`, which makes changing \
             `{{:<12}}` to `{{:<14}}` — a presentation decision — fail a test about behaviour"
        );
        assert!(
            check(
                &on_one_line(&["inactive", "KUBE"]),
                widened,
                "expect.stdout"
            )
            .is_empty(),
            "and order within the line is not checked either, for the same reason"
        );
    }

    #[test]
    fn a_failed_line_group_names_the_closest_line_and_what_it_lacked() {
        let diffs = check(&on_one_line(&["KUBE", "active"]), TABLE, "expect.stdout");

        assert_eq!(diffs[0].path, "expect.stdout.line_includes[0]");
        assert!(
            diffs[0].got.contains("line 2") && diffs[0].got.contains("KUBE"),
            "the line that came closest, by number and by content: {:?}",
            diffs[0].got
        );
        assert!(
            diffs[0].got.contains("\"active\""),
            "and what that line was missing, which is the whole diagnostic: {:?}",
            diffs[0].got
        );
    }

    /// Between two lines equally close, the one holding what the case asked about.
    #[test]
    fn the_closest_line_is_the_one_carrying_the_row_the_case_named() {
        let diffs = check(
            &on_one_line(&["DOCKER", "inactive"]),
            TABLE,
            "expect.stdout",
        );

        assert!(
            diffs[0].got.contains("DOCKER"),
            "line 2 holds `inactive` and line 3 holds `DOCKER`, so both miss exactly one value. The \
             row a reader wants named is the one they asked about, and the first value is the row \
             key in every table anyone writes: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn a_line_group_nothing_matches_at_all_shows_the_stream() {
        let diffs = check(&on_one_line(&["NOTHING", "HERE"]), TABLE, "expect.stdout");

        assert!(
            diffs[0].got.contains("MODULE"),
            "with no line sharing a single fragment there is no near miss worth pointing at, so the \
             stream itself is the better answer: {:?}",
            diffs[0].got
        );
    }

    /// A coloured status is one word once the escapes are gone, and not before.
    ///
    /// Worth asserting rather than assuming: a table that colours its statuses is the realistic case,
    /// and a whole-word comparison against `\e[32mactive\e[0m` matches nothing at all. It works because
    /// `check` strips first and every comparison sees the same text — but "it falls out of the order the
    /// functions happen to be in" is not a guarantee, and this is what makes it one.
    #[test]
    fn a_line_group_sees_the_stripped_text_like_everything_else() {
        let coloured = "MODULE STATUS\nKUBE \u{1b}[32mactive\u{1b}[0m\n";

        assert!(
            check(
                &ignoring_ansi(on_one_line(&["KUBE", "active"])),
                coloured,
                "expect.stdout"
            )
            .is_empty(),
            "the words are there once the colour is not"
        );
        assert_eq!(
            check(&on_one_line(&["KUBE", "active"]), coloured, "expect.stdout").len(),
            1,
            "and without `ignore_ansi` the word is `\\e[32mactive\\e[0m`, which is not `active` — the \
             same decision as everywhere: a case may legitimately assert that a colour is there"
        );
    }

    /// A group with nothing in it is satisfied by every line, so it is said rather than passed.
    #[test]
    fn an_empty_line_group_is_a_failure_rather_than_a_free_pass() {
        let diffs = check(&on_one_line(&[]), TABLE, "expect.stdout");

        assert_eq!(
            diffs.len(),
            1,
            "an assertion that cannot fail is the one thing this project refuses everywhere else"
        );
        assert!(
            diffs[0].expected.contains("at least one value"),
            "and it says what to write instead: {:?}",
            diffs[0]
        );
    }

    #[test]
    fn equals_refuses_the_substring_that_contains_would_have_accepted() {
        // From zanvil's report, verbatim: `printf 12` with `contains: ["2"]` passes. A case counting
        // lines and asserting `contains: ["2"]` therefore passes on a result of 12.
        assert!(
            check(&contains("2"), "12", "expect.stdout").is_empty(),
            "this is the behaviour being worked around, not a regression: `contains` is doing \
             exactly what it says"
        );

        let diffs = check(&equals("2"), "12", "expect.stdout");

        assert_eq!(diffs.len(), 1, "12 is not 2");
        assert_eq!(diffs[0].path, "expect.stdout.equals");
        assert_eq!(diffs[0].expected, "2");
        assert!(
            diffs[0].got.contains("12"),
            "both sides are known here, so the failure can show them: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn one_trailing_newline_is_ignored_on_either_side() {
        assert!(
            check(&equals("hello"), "hello\n", "expect.stdout").is_empty(),
            "a shell subject ends its output with a newline almost always and a case never writes \
             one. Comparing to the byte would fail every first attempt for a reason nothing in the \
             case explains"
        );
        assert!(
            check(&equals("hello\n"), "hello", "expect.stdout").is_empty(),
            "and symmetrically, so a case that does write it is not punished either"
        );
    }

    /// Multi-line output is where the two values stop being readable, so the report changes shape.
    ///
    /// A ten-line expectation rendered with `⏎` for every newline is one long line of glyphs, and the
    /// stream beside it is cut at 120 characters — the reader ends up hunting one wrong character
    /// across two mangled strings. Naming the line replaces that with a place to look.
    #[test]
    fn a_multi_line_difference_names_the_line_it_diverges_on() {
        let diffs = check(
            &equals("name: api\nversion: 1.2.4\nport: 8080"),
            "name: api\nversion: 1.2.3\nport: 8080",
            "expect.stdout",
        );

        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].expected, r#"line 2  "version: 1.2.4""#);
        assert_eq!(diffs[0].got, r#"line 2  "version: 1.2.3" (line 1 matched)"#);
    }

    /// How far they agreed, because `line 47 differs` does not say whether 1 to 46 are fine.
    #[test]
    fn a_divergence_deep_in_a_stream_says_how_much_matched() {
        let want: String = (1..=8).map(|n| format!("line {n}\n")).collect();
        let got = want.replace("line 6", "line six");

        let diffs = check(&equals(&want), &got, "expect.stdout");

        assert_eq!(diffs.len(), 1);
        assert!(
            diffs[0].got.contains("the first 5 lines matched"),
            "or the reader re-reads the whole stream to find out: {}",
            diffs[0].got
        );
    }

    /// A stream that stopped early is the commonest divergence of all.
    ///
    /// The alternative is an empty value on one side, which reads as "the subject wrote nothing" —
    /// the exact confusion that made `got` show the whole stream instead of its first line.
    #[test]
    fn a_stream_that_ends_too_soon_says_so_rather_than_showing_nothing() {
        let diffs = check(&equals("a\nb\nc\nd"), "a\nb\nc", "expect.stdout");

        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].expected, r#"line 4  "d""#);
        assert_eq!(diffs[0].got, "the stream ends after 3 lines");
    }

    /// And a stream that kept going says where, and how much of it there is.
    #[test]
    fn a_stream_that_writes_past_the_expectation_says_how_much_of_it_there_is() {
        let diffs = check(
            &equals("a\nb"),
            "a\nb\nwarning: deprecated\n",
            "expect.stdout",
        );

        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].expected, "nothing after line 2");
        assert_eq!(
            diffs[0].got,
            r#"line 3  "warning: deprecated" — 3 lines in all"#
        );
    }

    /// The forgiven trailing newline must not invent a divergence one past the end.
    ///
    /// `"a\nb\n".split('\n')` yields a third, empty element. Splitting without stripping it would
    /// report `line 3 expected nothing, got ""` on a stream that matches perfectly.
    #[test]
    fn a_trailing_newline_does_not_become_a_phantom_line() {
        assert!(
            check(&equals("a\nb"), "a\nb\n", "expect.stdout").is_empty(),
            "the newline `equals` forgives cannot come back as a line number"
        );
    }

    /// One long line cannot flood the report either.
    #[test]
    fn a_long_diverging_line_is_capped_like_any_other_value() {
        let diffs = check(
            &equals(&format!("first\n{}", "x".repeat(400))),
            &format!("first\n{}", "y".repeat(400)),
            "expect.stdout",
        );

        assert_eq!(diffs.len(), 1);
        assert!(
            diffs[0].got.contains("400 bytes in all"),
            "the length is named so the reader knows what was left out: {}",
            diffs[0].got
        );
        assert!(
            diffs[0].got.chars().count() < 200,
            "and one line of a report stays one line: {}",
            diffs[0].got
        );
    }

    #[test]
    fn a_second_trailing_newline_is_a_difference() {
        let diffs = check(&equals("hello"), "hello\n\n", "expect.stdout");

        assert_eq!(
            diffs.len(),
            1,
            "one newline is forgiven, not all trailing whitespace: two blank lines at the end may \
             be the bug being hunted, and swallowing them would make the assertion weaker than \
             whoever wrote it believes"
        );
    }

    #[test]
    fn a_whitespace_only_difference_says_so_rather_than_showing_two_identical_values() {
        let diffs = check(&equals("a b"), "a\tb", "expect.stdout");

        assert_eq!(diffs.len(), 1);
        assert!(
            diffs[0].got.contains("the same but for whitespace"),
            "a tab against a space renders identically in a report, and `expected a b, got a b` \
             sends the reader hunting a bug in the comparison: {:?}",
            diffs[0].got
        );
        assert!(
            diffs[0].got.contains("→"),
            "and the tab itself is visible, which is what makes the sentence actionable: {:?}",
            diffs[0].got
        );
    }

    /// A line as zanvil's log formatter really emits it: every field wrapped in its own codes, and
    /// the level padded so the columns line up — which is why the plain text has two spaces there.
    const COLOURED: &str = "\u{1b}[2m08:00:00.123\u{1b}[0m \u{1b}[1;32mINFO \u{1b}[0m Offset";

    fn ignoring_ansi(mut expectation: TextExpectation) -> TextExpectation {
        expectation.ignore_ansi = true;
        expectation
    }

    #[test]
    fn a_coloured_line_is_matched_on_its_words() {
        assert!(
            !check(
                &contains("08:00:00.123 INFO  Offset"),
                COLOURED,
                "expect.stdout"
            )
            .is_empty(),
            "without the key the escapes sit between the words and the substring is not there — \
             which is the behaviour being worked around, not a bug"
        );

        assert!(
            check(
                &ignoring_ansi(contains("08:00:00.123 INFO  Offset")),
                COLOURED,
                "expect.stdout"
            )
            .is_empty(),
            "the alternative was writing the escapes into the expectation, which works and is \
             unreadable — paying the first property to buy the assertion"
        );
    }

    #[test]
    fn stripping_is_off_unless_the_case_asks() {
        // The first thing worth asserting about a terminal tool: it does not colour when its output
        // is not a terminal. Stripping always would make this pass on coloured output.
        let diffs = check(&absent("\u{1b}["), COLOURED, "expect.stdout");

        assert_eq!(
            diffs.len(),
            1,
            "a case may legitimately prove a colour is absent, so stripping by default would \
             destroy that assertion silently"
        );
    }

    #[test]
    fn equals_and_absent_are_stripped_too() {
        assert!(
            check(
                &ignoring_ansi(equals("08:00:00.123 INFO  Offset")),
                COLOURED,
                "expect.stdout"
            )
            .is_empty(),
            "an equality on a coloured line is the case that most needs this: every escape has to \
             go from both sides or nothing matches"
        );
        assert!(
            check(&ignoring_ansi(absent("INFO")), COLOURED, "expect.stdout").len() == 1,
            "and `absent` sees the stripped text as well, or the same case would mean two things"
        );
    }

    #[test]
    fn a_failure_shows_the_stripped_text_rather_than_the_escapes() {
        let diffs = check(&ignoring_ansi(contains("nope")), COLOURED, "expect.stdout");

        assert!(
            !diffs[0].got.contains("\\e"),
            "showing escapes the case asked to ignore would explain nothing about why it failed: \
             {:?}",
            diffs[0].got
        );
        assert!(
            diffs[0].got.contains("INFO"),
            "what is shown is what was compared: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn a_window_title_goes_too_not_only_the_colours() {
        // OSC, terminated by BEL. A tool that colours usually also sets a title or emits a
        // hyperlink, so handling only CSI would move the problem one step along.
        let stream = "\u{1b}]0;deploying\u{7}done";

        assert!(
            check(&ignoring_ansi(equals("done")), stream, "expect.stdout").is_empty(),
            "and the next reader would have no idea why half the escapes went and half stayed"
        );
    }

    #[test]
    fn an_osc_terminated_by_string_terminator_is_handled() {
        let stream = "\u{1b}]8;;https://example.com\u{1b}\\link\u{1b}]8;;\u{1b}\\";

        assert!(
            check(&ignoring_ansi(equals("link")), stream, "expect.stdout").is_empty(),
            "a terminal hyperlink uses ESC \\ rather than BEL, and both bytes have to go or a \
             stray backslash survives into the comparison"
        );
    }

    #[test]
    fn text_with_no_escapes_is_untouched() {
        assert!(
            check(
                &ignoring_ansi(equals("plain text")),
                "plain text",
                "expect.stdout"
            )
            .is_empty(),
            "the key must cost nothing on a subject that never colours anything"
        );
    }

    #[test]
    fn equals_composes_with_the_other_two() {
        let expectation = TextExpectation {
            contains: vec!["ell".to_string()],
            absent: vec!["zzz".to_string()],
            line_includes: vec![vec!["hello".to_string()]],
            equals: Some("hello".to_string()),
            ignore_ansi: false,
        };

        assert!(
            check(&expectation, "hello", "expect.stdout").is_empty(),
            "nothing about `equals` replaces the others; a case may state all of them and they are \
             all checked"
        );
    }

    #[test]
    fn a_stream_that_is_only_invisible_bytes_does_not_read_as_an_empty_one() {
        // What `_ui_header` produces: a colour escape, a newline, then the content. The old
        // excerpt took the first line, which was the escape alone, and rendered as nothing.
        let stream = "\u{1b}[1;32m\n== lazygit ==\nconfig: /nowhere\n";

        let diffs = check(
            &contains("un-chemin-qui-nexiste-pas"),
            stream,
            "expect.stdout",
        );

        assert_eq!(diffs.len(), 1);
        assert!(
            !diffs[0].got.trim().is_empty(),
            "an empty `got` on a stream assertion says the subject wrote nothing, and this subject \
             wrote three lines. That reading cost a consumer most of a session on a case that had \
             no problem: {:?}",
            diffs[0].got
        );
        assert!(
            diffs[0].got.contains("lazygit"),
            "and what it wrote has to be in there, not just the first line: {:?}",
            diffs[0].got
        );
        assert!(
            diffs[0].got.contains("\\e"),
            "the escape is shown rather than interpreted, because a terminal hides exactly the \
             bytes a diagnostic most needs to show — and `contains:` failed on them: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn newlines_are_visible_so_the_report_keeps_one_line_per_failure() {
        let diffs = check(&contains("nope"), "first\nsecond", "expect.stdout");

        assert!(
            !diffs[0].got.contains('\n'),
            "the terminal report aligns `got` in a column; a real newline would break the layout \
             of every failure after it: {:?}",
            diffs[0].got
        );
        assert!(
            diffs[0].got.contains("first") && diffs[0].got.contains("second"),
            "both lines are still there: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn a_truly_empty_stream_says_so() {
        let diffs = check(&contains("anything"), "", "expect.stdout");

        assert_eq!(
            diffs[0].got, "(empty)",
            "the honest case has to stay honest, or the fix above would have traded one \
             misleading message for another"
        );
    }

    #[test]
    fn whitespace_only_is_distinguished_from_empty() {
        let diffs = check(&contains("anything"), "  \n\t\n ", "expect.stdout");

        assert!(
            diffs[0].got.contains("whitespace"),
            "a subject that wrote six bytes of blanks did write something, and a reader chasing a \
             missing newline needs to know: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn a_long_stream_is_capped_and_names_what_it_left_out() {
        let stream = "x".repeat(5_000);

        let diffs = check(&contains("nope"), &stream, "expect.stdout");

        assert!(
            diffs[0].got.len() < 400,
            "one long stream must not drown the failures under it: {}",
            diffs[0].got.len()
        );
        assert!(
            diffs[0].got.contains("5000 bytes in all"),
            "and the reader is told how much the subject really wrote, so a cap is never mistaken \
             for the whole output: {:?}",
            diffs[0].got
        );
    }

    #[test]
    fn an_absent_failure_still_shows_the_offending_line_rather_than_the_stream() {
        let stream = "line one\nhere is the ZSH_ENV_DIR problem\nline three";

        let diffs = check(&absent("ZSH_ENV_DIR"), stream, "expect.stdout");

        assert_eq!(diffs.len(), 1);
        assert!(
            diffs[0].got.contains("ZSH_ENV_DIR problem"),
            "the point of an `absent` failure is where the needle is: {:?}",
            diffs[0].got
        );
        assert!(
            !diffs[0].got.contains("line three"),
            "and showing the whole stream would bury it: {:?}",
            diffs[0].got
        );
    }
}
