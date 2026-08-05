//! Expectations on the files the subject wrote.
//!
//! This family matters as much as outgoing calls: the bug this project was scoped around
//! appeared in no output at all, only in a file dropped outside the repository.

use std::collections::BTreeMap;

use crate::TextExpectation;
use crate::iso::paths;
use crate::iso::snapshot::{FileChange, FileEffect};
use crate::verdict::{Diff, text};

/// Checks per-path expectations against what the subject wrote, rooting every path at `at`.
///
/// `at` rather than a hardcoded `expect`, for the reason `calls::check` needed the same correction one
/// release earlier: a `files:` broken inside an exchange was reported as `expect.files[…]`, sending the
/// reader to the case's own block. That one was found by a consumer reading the checks in a row; this one
/// was in the same list and neither of us looked.
pub fn check(
    expected: &BTreeMap<String, TextExpectation>,
    effects: &[FileEffect],
    defined: &BTreeMap<String, String>,
    at: &str,
) -> Vec<Diff> {
    let mut diffs = Vec::new();

    for (pattern, expectation) in expected {
        let prefix = format!("{at}.files[{pattern:?}]");

        let resolved = match paths::substitute(pattern, defined) {
            Ok(path) => path,
            Err(error) => {
                diffs.push(Diff {
                    path: prefix,
                    expected: "a resolvable path".to_string(),
                    got: error.to_string(),
                });
                continue;
            }
        };

        let Some(effect) = effects.iter().find(|effect| effect.path == resolved) else {
            diffs.push(Diff {
                path: prefix,
                expected: "written by the subject".to_string(),
                got: "not written".to_string(),
            });
            continue;
        };

        match &effect.content {
            Some(content) => diffs.extend(text::check(expectation, content, &prefix)),
            None => diffs.push(Diff {
                path: prefix,
                expected: "assertable text content".to_string(),
                got: format!(
                    "content not captured: {} bytes, and either not UTF-8 or over the cap",
                    effect.size
                ),
            }),
        }
    }

    diffs
}

/// Paths the case says must not have been touched, against what the subject touched.
///
/// **The gap this fills.** `no_new_files: true` asserts the subject wrote nothing at all, and `files`
/// fails a path that was *not* written — so between "everything" and "one specific file must exist" there
/// was no way to say "this one must not". The ordinary case is a subject that legitimately writes its own
/// log and must not touch your configuration.
///
/// A path naming a variable isolation does not define is a failure rather than a literal, and here that
/// is load-bearing rather than tidy: this assertion is negative, so a `$TYPO` would resolve to nothing,
/// match no effect and hold for ever.
pub fn not_written(
    paths: &[String],
    effects: &[FileEffect],
    defined: &BTreeMap<String, String>,
    at: &str,
) -> Vec<Diff> {
    let mut diffs = Vec::new();

    for pattern in paths {
        let prefix = format!("{at}.not_written[{pattern:?}]");

        let resolved = match paths::substitute(pattern, defined) {
            Ok(path) => path,
            Err(error) => {
                diffs.push(Diff {
                    path: prefix,
                    expected: "a resolvable path".to_string(),
                    got: error.to_string(),
                });
                continue;
            }
        };

        if let Some(effect) = effects.iter().find(|effect| effect.path == resolved) {
            diffs.push(Diff {
                path: prefix,
                expected: "not written".to_string(),
                got: match effect.change {
                    FileChange::Created => format!("created, {}", bytes(effect.size)),
                    FileChange::Modified => format!("modified, {}", bytes(effect.size)),
                    // Not `created, 0 bytes`, which would be a lie about a file that is gone — and a
                    // removal is the effect this assertion exists to catch most.
                    FileChange::Removed => "removed".to_string(),
                },
            });
        }
    }

    diffs
}

/// A size with its unit agreeing with it. `1 bytes` reads as a bug in the report.
fn bytes(size: u64) -> String {
    match size {
        1 => "1 byte".to_string(),
        many => format!("{many} bytes"),
    }
}

/// The files the subject wrote that the case says nothing about.
///
/// Offered as help, never as a failure. It is often where you discover what you should have
/// been asserting — and a case that had to enumerate every incidental file would be
/// unwritable.
///
/// A path under `not_written` counts as mentioned. It was written, so it is already a failure of its
/// own; listing it again as something the case said nothing about would be untrue twice over.
pub fn unmentioned(
    expected: &BTreeMap<String, TextExpectation>,
    forbidden: &[String],
    effects: &[FileEffect],
    defined: &BTreeMap<String, String>,
) -> Vec<String> {
    let mentioned: Vec<_> = expected
        .keys()
        .chain(forbidden)
        .filter_map(|pattern| paths::substitute(pattern, defined).ok())
        .collect();

    effects
        .iter()
        .filter(|effect| !mentioned.contains(&effect.path))
        .map(|effect| effect.path.to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iso::snapshot::FileChange;

    fn defined() -> BTreeMap<String, String> {
        [("HOME".to_string(), "/iso".to_string())]
            .into_iter()
            .collect()
    }

    fn effect(relative: &str, content: Option<&str>) -> FileEffect {
        FileEffect {
            path: std::path::PathBuf::from(relative),
            change: FileChange::Created,
            size: content.map_or(0, |body| body.len() as u64),
            content: content.map(String::from),
        }
    }

    fn expected(
        path: &str,
        contains: &[&str],
        absent: &[&str],
    ) -> BTreeMap<String, TextExpectation> {
        [(
            path.to_string(),
            TextExpectation {
                contains: contains
                    .iter()
                    .map(|needle| (*needle).to_string())
                    .collect(),
                absent: absent.iter().map(|needle| (*needle).to_string()).collect(),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect()
    }

    #[test]
    fn a_deposited_file_satisfying_both_families_passes() {
        let effects = vec![effect(
            "Library/k9s/plugins.yaml",
            Some("scriptPath: /iso/scripts/fmt.zsh\nname: log-view-pod\n"),
        )];
        let diffs = check(
            &expected(
                "$HOME/Library/k9s/plugins.yaml",
                &["log-view-pod"],
                &["$ZANVIL_DIR", "ZSH_ENV"],
            ),
            &effects,
            &defined(),
            "expect",
        );

        assert!(diffs.is_empty(), "diffs: {diffs:?}");
    }

    #[test]
    fn an_unresolved_variable_left_in_a_deposited_file_is_caught() {
        let effects = vec![effect(
            "Library/k9s/plugins.yaml",
            Some("scriptPath: $ZSH_ENV_DIR/scripts/fmt.zsh\n"),
        )];
        let diffs = check(
            &expected("$HOME/Library/k9s/plugins.yaml", &[], &["ZSH_ENV"]),
            &effects,
            &defined(),
            "expect",
        );

        assert_eq!(diffs.len(), 1);
        assert_eq!(
            diffs[0].path, "expect.files[\"$HOME/Library/k9s/plugins.yaml\"].absent[0]",
            "the diff path must quote the case's own spelling, not the resolved one: that is \
             what a reader searches for in the file"
        );
        assert!(diffs[0].got.contains("ZSH_ENV_DIR"));
    }

    #[test]
    fn a_file_the_subject_never_wrote_is_a_failure_naming_the_path() {
        let diffs = check(
            &expected("$HOME/never-written.yaml", &["anything"], &[]),
            &[],
            &defined(),
            "expect",
        );

        assert_eq!(diffs.len(), 1);
        assert!(
            diffs[0].got.contains("not written"),
            "a case asserting about a file that was never written must say that, not that a \
             substring was missing: {:?}",
            diffs[0]
        );
    }

    #[test]
    fn a_file_whose_content_could_not_be_captured_fails_loudly() {
        let effects = vec![effect("blob.bin", None)];
        let diffs = check(
            &expected("blob.bin", &["needle"], &[]),
            &effects,
            &defined(),
            "expect",
        );

        assert_eq!(diffs.len(), 1);
        assert!(
            diffs[0].got.contains("content"),
            "asserting text against a binary or oversized file must fail explaining why, \
             never pass silently: {:?}",
            diffs[0]
        );
    }

    #[test]
    fn an_unknown_variable_in_an_expected_path_is_reported_as_a_case_error() {
        let diffs = check(
            &expected("$TYPO/foo", &["x"], &[]),
            &[],
            &defined(),
            "expect",
        );

        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].got.contains("TYPO"));
    }

    /// A path the subject must not touch, next to one it legitimately writes.
    ///
    /// **This is what was missing between the two.** `no_new_files: true` says the subject wrote nothing
    /// at all, and `files` fails a path that was *not* written — so the ordinary shape, a tool that
    /// writes its own log and must leave your configuration alone, could not be asserted.
    #[test]
    fn a_path_the_subject_must_leave_alone_is_asserted_beside_one_it_writes() {
        let wrote_its_log = vec![effect(".cache/mytool/run.log", Some("done"))];

        assert!(
            not_written(
                &["$HOME/.config/mytool/config.toml".to_string()],
                &wrote_its_log,
                &defined(),
                "expect",
            )
            .is_empty(),
            "the subject wrote its log and nothing else, so the configuration was left alone — \
             which `no_new_files: true` would have called a failure"
        );

        let diffs = not_written(
            &["$HOME/.config/mytool/config.toml".to_string()],
            &[effect(".config/mytool/config.toml", Some("meddled"))],
            &defined(),
            "expect",
        );

        assert_eq!(
            diffs[0].path, "expect.not_written[\"$HOME/.config/mytool/config.toml\"]",
            "quoting the case's own spelling rather than the resolved path: that is what a reader \
             searches the file for"
        );
        assert_eq!(diffs[0].expected, "not written");
        assert!(
            diffs[0].got.contains("created") && diffs[0].got.contains("bytes"),
            "and *how* it was touched, since created, modified and removed send a reader to three \
             different places: {:?}",
            diffs[0]
        );
    }

    #[test]
    fn a_removed_file_says_removed_rather_than_a_size() {
        let deleted = FileEffect {
            path: std::path::PathBuf::from(".ssh/config"),
            change: FileChange::Removed,
            size: 0,
            content: None,
        };

        let diffs = not_written(
            &["$HOME/.ssh/config".to_string()],
            &[deleted],
            &defined(),
            "expect",
        );

        assert_eq!(
            diffs[0].got, "removed",
            "`created, 0 bytes` would be a lie about a file that is gone, and this is the case the \
             assertion exists for most"
        );
    }

    /// A path naming nothing is refused, and here that is the assertion rather than tidiness.
    #[test]
    fn an_unknown_variable_in_a_forbidden_path_cannot_pass_vacuously() {
        let diffs = not_written(
            &["$HOEM/.config/mytool/config.toml".to_string()],
            &[effect(".config/mytool/config.toml", Some("meddled"))],
            &defined(),
            "expect",
        );

        assert_eq!(diffs.len(), 1);
        assert!(
            diffs[0].got.contains("HOEM"),
            "a negative assertion on a path that resolves to nothing matches no effect and holds \
             for ever — the subject above meddled with the file and the case would have been \
             green: {:?}",
            diffs[0]
        );
    }

    #[test]
    fn a_forbidden_path_that_was_written_is_not_also_listed_as_unmentioned() {
        let effects = vec![effect(".config/mytool/config.toml", Some("meddled"))];
        let forbidden = ["$HOME/.config/mytool/config.toml".to_string()];

        assert!(
            unmentioned(&BTreeMap::new(), &forbidden, &effects, &defined()).is_empty(),
            "it is already a failure of its own, and the case did say something about it. Listing \
             it as ignored as well would be untrue twice over"
        );
    }

    #[test]
    fn files_the_case_says_nothing_about_are_listed_as_help_not_as_failures() {
        let effects = vec![
            effect("mentioned.yaml", Some("ok")),
            effect(".cache/incidental.db", Some("noise")),
        ];
        let asserted = expected("mentioned.yaml", &["ok"], &[]);

        assert!(
            check(&asserted, &effects, &defined(), "expect").is_empty(),
            "an unmentioned file must not fail a case"
        );
        assert_eq!(
            unmentioned(&asserted, &[], &effects, &defined()),
            vec![".cache/incidental.db".to_string()],
            "listing what the case ignored is how you discover what you should have been \
             asserting"
        );
    }
}
