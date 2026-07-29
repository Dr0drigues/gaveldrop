//! Noticing that something changed, so a run can follow a save.
//!
//! **Polling, not a filesystem notification API.** `notify` is the obvious crate and it is at
//! `9.0.0-rc.4`; depending on a release candidate for a development convenience is the wrong trade.
//! And the cost of polling is a `stat` per watched file every few hundred milliseconds — for a project
//! of a few dozen cases that is nothing, and it behaves identically on both platforms with no
//! per-backend surprises.
//!
//! If a project ever has thousands of cases, that is the moment to reconsider, and this module is the
//! only place that would change.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What every watched file looked like at one moment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fingerprints {
    seen: BTreeMap<PathBuf, Print>,
}

/// Enough of a file to tell it changed.
///
/// Modification time **and** length. Time alone misses an edit landing inside the same clock tick,
/// which a fast editor doing write-then-rename can produce; length alone misses a change that keeps
/// the size. Neither is airtight — a same-length edit within one tick is invisible — and the failure
/// mode is a save that does not trigger a run, which a second save fixes. Hashing every file on every
/// poll would buy certainty nobody needs for that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Print {
    at: Option<SystemTime>,
    len: u64,
}

impl Fingerprints {
    /// Looks at every path now.
    ///
    /// A path that cannot be read is recorded as absent rather than skipped, so deleting a file counts
    /// as a change — otherwise removing a case would leave the last run on screen forever.
    pub fn take(paths: &[PathBuf]) -> Self {
        Self {
            seen: paths
                .iter()
                .map(|path| (path.clone(), print_of(path)))
                .collect(),
        }
    }

    /// The paths that differ from `earlier`, including ones that appeared or vanished.
    pub fn changed_since(&self, earlier: &Self) -> Vec<PathBuf> {
        let mut changed: Vec<PathBuf> = self
            .seen
            .iter()
            .filter(|(path, now)| earlier.seen.get(*path) != Some(now))
            .map(|(path, _)| path.clone())
            .collect();

        changed.extend(
            earlier
                .seen
                .keys()
                .filter(|path| !self.seen.contains_key(*path))
                .cloned(),
        );

        changed.sort();
        changed.dedup();
        changed
    }
}

/// What a run should cover after `changed`.
///
/// A **case document** that changed reruns that case alone, which is the whole point: editing an
/// expectation and seeing it re-evaluated in under a second is what makes a watch worth having.
///
/// Anything else — a sourced shell file, a service, a hook — reruns **everything**. Tracing which
/// cases depend on which file would mean knowing what a `serve:` command reads, which is not
/// knowable without running it. Guessing would silently skip the case that mattered, and the whole
/// suite is what a wrong guess costs to avoid.
pub fn affected(changed: &[PathBuf], cases: &[PathBuf]) -> Scope {
    if changed.is_empty() {
        return Scope::Nothing;
    }

    let touched: Vec<PathBuf> = changed
        .iter()
        .filter(|path| cases.contains(path))
        .cloned()
        .collect();

    if touched.len() == changed.len() {
        Scope::Cases(touched)
    } else {
        Scope::Everything
    }
}

/// How much to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Nothing changed.
    Nothing,
    /// Only these case documents changed.
    Cases(Vec<PathBuf>),
    /// Something a case depends on changed, and which cases is not knowable.
    Everything,
}

/// A file's fingerprint, or the absence of one.
fn print_of(path: &Path) -> Print {
    match std::fs::metadata(path) {
        Ok(data) => Print {
            at: data.modified().ok(),
            len: data.len(),
        },
        Err(_) => Print { at: None, len: 0 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn an_unchanged_file_is_not_reported() {
        let dir = tempfile::tempdir().unwrap();
        let one = write(&dir, "one.yaml", "name: a\n");
        let before = Fingerprints::take(std::slice::from_ref(&one));

        assert!(
            Fingerprints::take(std::slice::from_ref(&one))
                .changed_since(&before)
                .is_empty(),
            "a poll that reports a change every time would rerun the suite forever"
        );
    }

    #[test]
    fn an_edited_file_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let one = write(&dir, "one.yaml", "name: a\n");
        let before = Fingerprints::take(std::slice::from_ref(&one));

        write(&dir, "one.yaml", "name: a\nweight: 1\n");

        assert_eq!(
            Fingerprints::take(std::slice::from_ref(&one)).changed_since(&before),
            vec![one]
        );
    }

    #[test]
    fn a_deleted_file_counts_as_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let one = write(&dir, "one.yaml", "name: a\n");
        let before = Fingerprints::take(std::slice::from_ref(&one));
        std::fs::remove_file(&one).unwrap();

        assert_eq!(
            Fingerprints::take(std::slice::from_ref(&one)).changed_since(&before),
            vec![one],
            "removing a case must not leave the last run on screen forever"
        );
    }

    #[test]
    fn a_file_that_appeared_counts_as_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let one = dir.path().join("later.yaml");
        let before = Fingerprints::take(std::slice::from_ref(&one));

        write(&dir, "later.yaml", "name: a\n");

        assert_eq!(
            Fingerprints::take(std::slice::from_ref(&one)).changed_since(&before),
            vec![one],
            "writing a new case is the most common thing to do while watching"
        );
    }

    #[test]
    fn editing_one_case_reruns_that_case_alone() {
        let cases = vec![PathBuf::from("a.yaml"), PathBuf::from("b.yaml")];

        assert_eq!(
            affected(&[PathBuf::from("a.yaml")], &cases),
            Scope::Cases(vec![PathBuf::from("a.yaml")]),
            "editing an expectation and seeing it re-evaluated in under a second is the whole \
             point of watching"
        );
    }

    #[test]
    fn editing_something_a_case_depends_on_reruns_everything() {
        let cases = vec![PathBuf::from("a.yaml")];

        assert_eq!(
            affected(&[PathBuf::from("functions/ui.zsh")], &cases),
            Scope::Everything,
            "tracing which cases read which file would mean knowing what a `serve:` command opens, \
             which is not knowable without running it. Guessing would silently skip the case that \
             mattered"
        );
    }

    #[test]
    fn a_mixed_change_reruns_everything_rather_than_the_cases_it_recognised() {
        let cases = vec![PathBuf::from("a.yaml")];

        assert_eq!(
            affected(
                &[PathBuf::from("a.yaml"), PathBuf::from("functions/ui.zsh")],
                &cases
            ),
            Scope::Everything,
            "running only the case it recognised would report a green suite while the shell file \
             every other case sources had just changed"
        );
    }

    #[test]
    fn nothing_changed_is_its_own_answer() {
        assert_eq!(affected(&[], &[PathBuf::from("a.yaml")]), Scope::Nothing);
    }
}
