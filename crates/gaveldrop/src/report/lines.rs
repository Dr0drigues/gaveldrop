//! Resolving an assertion path to a line in the case document.
//!
//! `serde_yaml_ng` reports positions only in its **errors**; a document that parsed carries no spans.
//! So the provenance is recovered by re-reading the file rather than kept at load time — which is what
//! `ARCHITECTURE.md` planned for when it decided to keep assertion paths: going from a path to a line
//! is easy, whereas reconstructing a provenance you did not keep is not.
//!
//! **This degrades and never fails.** A path resolving three segments out of five reports the deepest
//! line it reached, and one resolving nothing reports line 1. An annotation on an approximate line is
//! useful; a case failing because a reporting detail returned an error is not.

/// The 1-indexed line `path` points at, as best it can be found.
pub fn locate(document: &str, path: &str) -> usize {
    let mut deepest = 1;
    let mut from = 0;
    let mut indent = 0;

    for segment in segments(path) {
        let Some(reached) = find(document, &segment, from, indent) else {
            return deepest;
        };

        deepest = reached.line;

        // A segment that only got part of the way stops the walk. `expect.stdout.absent[0]` reached
        // `stdout:` and no further, and applying the `[0]` anyway would land on the first element of
        // whatever sequence comes next — `contains:`, here — which is a confidently wrong line rather
        // than an honestly approximate one.
        if !reached.complete {
            return deepest;
        }

        from = reached.line;
        indent = reached.indent;
    }

    deepest
}

/// How far into the document a segment got.
struct Reached {
    /// The line it landed on.
    line: usize,
    /// How far that line is indented.
    indent: usize,
    /// Whether the whole segment resolved, or only a prefix of it.
    complete: bool,
}

/// One step of an assertion path.
#[derive(Debug, PartialEq, Eq)]
enum Segment {
    /// A mapping key, taken whole — it may contain dots, as `data.order.id` does.
    Key(String),
    /// The nth element of the sequence under the preceding key, 0-indexed.
    Nth(usize),
}

/// Splits a path into the steps to follow.
///
/// Three shapes the verdict produces, and each needs its own handling:
///
/// - `steps[0] "creates an order".status` — the index selects a sequence element and the quoted name
///   is **not** a key to descend into. It is there for a human reading the failure.
/// - `expect.files["$HOME/orders.log"]` — a bracketed key is taken whole, because a file path
///   contains dots and splitting on them would make it unresolvable.
/// - `expect.json.data.order.id.contains[0]` — `data.order.id` is *one* YAML key containing dots. The
///   resolver tries the longest remaining run as a key before splitting further, which is what
///   [`find`] does.
fn segments(path: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut rest = path;

    while !rest.is_empty() {
        if let Some((before, after)) = rest.split_once('[') {
            let (inside, tail) = match after.split_once(']') {
                Some(pair) => pair,
                None => (after, ""),
            };

            if !before.is_empty() {
                push_dotted(before.trim_start_matches('.'), &mut out);
            }

            match inside.trim().parse::<usize>() {
                Ok(nth) => out.push(Segment::Nth(nth)),
                Err(_) => out.push(Segment::Key(inside.trim_matches('"').to_string())),
            }

            rest = strip_quoted_name(tail);
            continue;
        }

        push_dotted(rest.trim_start_matches('.'), &mut out);
        break;
    }

    out
}

/// Drops the human-facing `"name"` a step path carries after its index.
fn strip_quoted_name(tail: &str) -> &str {
    let trimmed = tail.trim_start();
    match trimmed.strip_prefix('"') {
        Some(after) => match after.split_once('"') {
            Some((_, rest)) => rest,
            None => "",
        },
        None => trimmed,
    }
}

/// Pushes a dotted run as one key, letting [`find`] split it if the whole does not match.
fn push_dotted(run: &str, out: &mut Vec<Segment>) {
    if !run.is_empty() {
        out.push(Segment::Key(run.to_string()));
    }
}

/// The line a segment sits on, searching after `from` and deeper than `indent`.
///
/// A `Key` whose whole text does not appear is retried on its first dotted component, and the
/// remainder is pushed back by returning the line of that component — which lets
/// `expect.json.data.order.id.contains[0]` resolve without the caller knowing which dots were part of
/// a key and which were separators.
fn find(document: &str, segment: &Segment, from: usize, indent: usize) -> Option<Reached> {
    match segment {
        Segment::Key(key) => longest_key(document, key, from, indent),
        Segment::Nth(nth) => nth_element(document, *nth, from, indent).map(|(line, at)| Reached {
            line,
            indent: at,
            complete: true,
        }),
    }
}

/// Finds `key`, then its dotted head, then that head's head — longest first.
fn longest_key(document: &str, key: &str, from: usize, indent: usize) -> Option<Reached> {
    let mut attempt = key;

    loop {
        if let Some((line, at)) = key_line(document, attempt, from, indent) {
            let rest = key
                .strip_prefix(attempt)
                .unwrap_or("")
                .trim_start_matches('.');
            if rest.is_empty() {
                return Some(Reached {
                    line,
                    indent: at,
                    complete: true,
                });
            }

            // The remainder failing does not undo what was reached: `expect.stdout.absent` with no
            // `absent:` in the document still knows where `stdout:` was, and pointing there beats
            // pointing at line 1. It is reported as **incomplete**, so the caller stops rather than
            // applying the rest of the path to the wrong parent.
            return Some(longest_key(document, rest, line, at).unwrap_or(Reached {
                line,
                indent: at,
                complete: false,
            }));
        }

        match attempt.rsplit_once('.') {
            Some((head, _)) => attempt = head,
            None => return None,
        }
    }
}

/// The line where `key:` appears, deeper than `indent` and after `from`.
fn key_line(document: &str, key: &str, from: usize, indent: usize) -> Option<(usize, usize)> {
    for (offset, text) in document.lines().enumerate().skip(from) {
        let at = indentation(text);
        let bare = text.trim_start();

        if bare.is_empty() || bare.starts_with('#') {
            continue;
        }
        if from > 0 && at <= indent && !bare.starts_with('-') {
            return None;
        }

        let candidate = bare.trim_start_matches("- ");
        for spelling in [
            format!("{key}:"),
            format!("\"{key}\":"),
            format!("'{key}':"),
        ] {
            if candidate.starts_with(&spelling) {
                return Some((offset + 1, at));
            }
        }
    }

    None
}

/// The line of the nth sequence element after `from`.
fn nth_element(document: &str, nth: usize, from: usize, indent: usize) -> Option<(usize, usize)> {
    let mut seen = 0;

    for (offset, text) in document.lines().enumerate().skip(from) {
        let at = indentation(text);
        let bare = text.trim_start();

        if bare.is_empty() || bare.starts_with('#') {
            continue;
        }
        if !bare.starts_with("- ") && !bare.starts_with('-') {
            if at <= indent && from > 0 && offset + 1 > from {
                return None;
            }
            continue;
        }

        if seen == nth {
            return Some((offset + 1, at));
        }
        seen += 1;
    }

    None
}

/// How far a line is indented.
fn indentation(text: &str) -> usize {
    text.len() - text.trim_start().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASE: &str = r#"name: an-order-is-created
weight: 8
setup:
  serve: ["python3", "app.py"]
expect:
  exit_code: 0
  stdout:
    contains:
      - "listening"
      - "ready"
  files:
    "$HOME/orders.log":
      contains: ["created"]
steps:
  - name: creates an order
    expect:
      status: 201
      json:
        data.order.id: { contains: ["7"] }
"#;

    fn line_of(path: &str) -> usize {
        locate(CASE, path)
    }

    #[test]
    fn a_top_level_key_resolves_to_its_own_line() {
        assert_eq!(line_of("setup"), 3);
    }

    #[test]
    fn a_nested_key_resolves_past_its_parents() {
        assert_eq!(line_of("expect.exit_code"), 6);
    }

    #[test]
    fn an_indexed_element_resolves_to_the_element_not_the_list() {
        assert_eq!(
            line_of("expect.stdout.contains[1]"),
            10,
            "an annotation on `contains:` when the second entry is the broken one sends the reader \
             to the right block and the wrong line"
        );
    }

    #[test]
    fn a_quoted_key_with_a_dot_in_it_is_not_split_on_that_dot() {
        assert_eq!(
            line_of("expect.files[\"$HOME/orders.log\"]"),
            12,
            "a file path contains dots. Splitting the path on every dot would make this \
             unresolvable, which is why a bracketed segment is taken whole"
        );
    }

    #[test]
    fn a_step_prefix_resolves_inside_that_step() {
        assert_eq!(
            line_of("steps[0] \"creates an order\".status"),
            17,
            "the name is part of the path the verdict produces, and it must not be mistaken for a \
             key to descend into"
        );
    }

    #[test]
    fn a_dotted_json_path_is_a_single_key_here() {
        assert_eq!(
            line_of("steps[0] \"creates an order\".json.data.order.id.contains[0]"),
            19,
            "`data.order.id` is one YAML key containing dots, not three levels. The resolver must \
             try the longest key first"
        );
    }

    #[test]
    fn a_path_that_stops_resolving_reports_the_deepest_line_it_reached() {
        assert_eq!(
            line_of("expect.stdout.absent[0]"),
            7,
            "`absent` is not in this document. Pointing at `stdout:` is useful; failing, or \
             pointing at line 1, is not"
        );
    }

    #[test]
    fn a_path_matching_nothing_at_all_is_line_one_rather_than_an_error() {
        assert_eq!(
            line_of("nowhere.at.all"),
            1,
            "an annotation on the wrong line is a cosmetic problem; a case failing because a \
             reporting detail returned an error is not"
        );
    }
}
