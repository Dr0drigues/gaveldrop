//! Listing the cases a project has, for something other than a person to read.
//!
//! An editor's test interface needs the tree **before** anything runs: a name to show, a file and a
//! line to jump to, a weight to sort by. It cannot get that from a run, because the point is to draw
//! the list first and fill in verdicts afterwards.
//!
//! Deliberately separate from the reports. A report answers "what happened"; this answers "what is
//! there", and conflating them would make an editor run the suite to draw a tree.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::report::lines;
use crate::{Case, CaseError};

/// One case, as something outside gaveldrop needs to see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    /// The case's name, which is also its identifier in every report.
    pub name: String,
    /// Where the document is, relative to wherever discovery started.
    pub path: PathBuf,
    /// The line the case's `name:` sits on, so an editor can jump to it.
    pub line: usize,
    /// How much this case matters, for sorting a list by consequence.
    pub weight: u32,
    /// Whether it is a declared known failure.
    pub allow_fail: bool,
    /// How many exchanges it performs, or `0` for a single invocation.
    pub steps: usize,
}

/// A case that could not be read, reported rather than dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unreadable {
    /// The file that would not load.
    pub path: PathBuf,
    /// Why, in the loader's own words.
    pub reason: String,
}

/// Everything discovery found, readable and not.
///
/// A broken document is **listed**, not skipped. An editor showing a tree with one case silently
/// missing is worse than one showing it greyed out with the parse error attached — the second tells
/// you where to look.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discovered {
    /// The cases that loaded.
    pub cases: Vec<Found>,
    /// The files that did not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unreadable: Vec<Unreadable>,
}

/// Reads every path into something machine-readable.
pub fn inspect(paths: &[PathBuf]) -> Discovered {
    let mut discovered = Discovered::default();

    for path in paths {
        match read(path) {
            Ok(found) => discovered.cases.push(found),
            Err(reason) => discovered.unreadable.push(Unreadable {
                path: path.clone(),
                reason,
            }),
        }
    }

    discovered
}

/// One case, or why it could not be read.
fn read(path: &Path) -> Result<Found, String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let case = Case::load_str(&text, path).map_err(|error: CaseError| error.to_string())?;

    Ok(Found {
        line: lines::locate(&text, "name"),
        name: case.name,
        path: path.to_path_buf(),
        weight: case.weight,
        allow_fail: case.allow_fail,
        steps: case.steps.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).unwrap();
        }
        dir
    }

    fn paths_in(dir: &tempfile::TempDir, names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(|name| dir.path().join(name)).collect()
    }

    const ONE: &str = "name: an-order-is-created\nweight: 8\nsetup:\n  run: [\"true\"]\nexpect:\n  exit_code: 0\n";

    #[test]
    fn a_case_is_found_with_everything_an_editor_needs_to_draw_it() {
        let dir = project(&[("one.yaml", ONE)]);
        let found = inspect(&paths_in(&dir, &["one.yaml"]));

        assert_eq!(found.cases.len(), 1);
        let case = &found.cases[0];
        assert_eq!(case.name, "an-order-is-created");
        assert_eq!(case.weight, 8);
        assert_eq!(
            case.line, 1,
            "the line of `name:`, so a click in a test tree lands on the case rather than on the \
             top of the file"
        );
        assert!(!case.allow_fail);
        assert_eq!(case.steps, 0);
    }

    #[test]
    fn a_case_with_steps_reports_how_many() {
        let dir = project(&[(
            "stepped.yaml",
            "name: stepped\nweight: 1\nsetup:\n  serve: [\"true\"]\nexpect: {}\nsteps:\n  - expect: {}\n  - expect: {}\n",
        )]);

        assert_eq!(
            inspect(&paths_in(&dir, &["stepped.yaml"])).cases[0].steps,
            2,
            "an editor showing a case with two exchanges as a single node hides half of what it \
             does"
        );
    }

    #[test]
    fn a_broken_document_is_listed_rather_than_dropped() {
        let dir = project(&[("one.yaml", ONE), ("broken.yaml", "name: [this is not\n")]);
        let found = inspect(&paths_in(&dir, &["one.yaml", "broken.yaml"]));

        assert_eq!(
            found.cases.len(),
            1,
            "the readable one still appears: one broken document must not empty the tree"
        );
        assert_eq!(found.unreadable.len(), 1);
        assert!(
            found.unreadable[0].reason.contains("broken.yaml"),
            "a tree with a case silently missing is worse than one showing it greyed out with the \
             reason attached: {:?}",
            found.unreadable[0].reason
        );
    }

    #[test]
    fn a_file_that_does_not_exist_is_unreadable_rather_than_a_panic() {
        let dir = project(&[]);
        let found = inspect(&paths_in(&dir, &["absent.yaml"]));

        assert_eq!(found.unreadable.len(), 1);
        assert!(found.cases.is_empty());
    }

    #[test]
    fn discovery_is_json_and_round_trips() {
        let dir = project(&[("one.yaml", ONE)]);
        let found = inspect(&paths_in(&dir, &["one.yaml"]));

        let text = serde_json::to_string(&found).unwrap();
        let back: Discovered = serde_json::from_str(&text).unwrap();

        assert_eq!(
            back, found,
            "an editor plugin reads this over a pipe, so what goes out has to come back identical"
        );
    }

    #[test]
    fn nothing_found_is_an_empty_list_rather_than_an_error() {
        assert_eq!(inspect(&[]), Discovered::default());
    }
}
