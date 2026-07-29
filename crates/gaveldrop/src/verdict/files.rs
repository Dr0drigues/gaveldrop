//! Expectations on the files the subject wrote.
//!
//! This family matters as much as outgoing calls: the bug this project was scoped around
//! appeared in no output at all, only in a file dropped outside the repository.

use std::collections::BTreeMap;

use crate::TextExpectation;
use crate::iso::paths;
use crate::iso::snapshot::FileEffect;
use crate::verdict::{Diff, text};

/// Checks per-path expectations against what the subject wrote.
pub fn check(
    expected: &BTreeMap<String, TextExpectation>,
    effects: &[FileEffect],
    defined: &BTreeMap<String, String>,
) -> Vec<Diff> {
    let mut diffs = Vec::new();

    for (pattern, expectation) in expected {
        let prefix = format!("expect.files[{pattern:?}]");

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

/// The files the subject wrote that the case says nothing about.
///
/// Offered as help, never as a failure. It is often where you discover what you should have
/// been asserting — and a case that had to enumerate every incidental file would be
/// unwritable.
pub fn unmentioned(
    expected: &BTreeMap<String, TextExpectation>,
    effects: &[FileEffect],
    defined: &BTreeMap<String, String>,
) -> Vec<String> {
    let mentioned: Vec<_> = expected
        .keys()
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
        let diffs = check(&expected("$TYPO/foo", &["x"], &[]), &[], &defined());

        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].got.contains("TYPO"));
    }

    #[test]
    fn files_the_case_says_nothing_about_are_listed_as_help_not_as_failures() {
        let effects = vec![
            effect("mentioned.yaml", Some("ok")),
            effect(".cache/incidental.db", Some("noise")),
        ];
        let asserted = expected("mentioned.yaml", &["ok"], &[]);

        assert!(
            check(&asserted, &effects, &defined()).is_empty(),
            "an unmentioned file must not fail a case"
        );
        assert_eq!(
            unmentioned(&asserted, &effects, &defined()),
            vec![".cache/incidental.db".to_string()],
            "listing what the case ignored is how you discover what you should have been \
             asserting"
        );
    }
}
