//! The case documents, for a renderer that points at the line an assertion came from.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::Case;
use crate::report::lines;

/// Every case's file and text, by case name.
///
/// **Read here rather than carried on `Outcome`.** An outcome is about the verdict — a name, a weight,
/// the assertions that did not hold — and widening it so a renderer can find a file would put reporting
/// concerns inside the thing being reported. The cost is reading each case once more, which is nothing
/// beside running it.
///
/// Shared by the two renderers that point at a line: the workflow annotations, which had this to
/// themselves, and the terminal, which now prints `--> path:line` beside each assertion. One loader
/// because a second would drift, and a report disagreeing with an annotation about where a failure lives
/// is worse than neither saying.
#[derive(Default)]
pub struct Sources {
    documents: BTreeMap<String, (PathBuf, String)>,
}

/// Where one assertion sits: the file, and the line.
///
/// No text of the line, and that was a deliberate subtraction after looking at real output. Quoting it
/// the way a compiler quotes a source span reads as `contains: ["au revoir"]` beside
/// `expected  contains "au revoir"` — a restatement. A compiler needs the span because it is the only
/// way to know *which* token; here the assertion path already names it. The day something wants the
/// text — a plugin drawing a squiggle — this is where it grows.
pub struct Located<'a> {
    /// The case's file.
    pub path: &'a Path,
    /// The line the assertion's path reached, 1-indexed.
    pub line: usize,
}

impl Located<'_> {
    /// `path:line`, the form a terminal turns into a link and an editor accepts as an argument.
    pub fn reference(&self) -> String {
        format!("{}:{}", self.path.display(), self.line)
    }
}

impl Sources {
    /// Loads every case among `paths`.
    ///
    /// A path that will not load is skipped rather than fatal: such a case fails for its own reasons and
    /// says so, and refusing to locate the others would let one broken document silence a whole report.
    pub fn load(paths: &[PathBuf]) -> Self {
        let mut documents = BTreeMap::new();

        for path in paths {
            if let Ok(text) = std::fs::read_to_string(path)
                && let Ok(case) = Case::load_str(&text, path)
            {
                documents.insert(case.name, (path.clone(), text));
            }
        }

        Self { documents }
    }

    /// Where `assertion` sits in `case`, or nothing when the case is not among the loaded ones.
    ///
    /// Absent rather than a guess: a case that failed to load, or one a consumer's own runner produced
    /// from somewhere else, has no document to walk — and a confident `:1` would send the reader to the
    /// wrong place, which is worse than sending them nowhere.
    pub fn locate(&self, case: &str, assertion: &str) -> Option<Located<'_>> {
        let (path, document) = self.documents.get(case)?;
        let line = lines::locate(document, assertion);

        Some(Located { path, line })
    }

    /// True when nothing was loaded, so a renderer can skip the lookup entirely.
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = "name: an-order\nweight: 5\nsetup:\n  run: [\"true\"]\nexpect:\n  stdout:\n    contains:\n      - \"created\"\n";

    fn loaded() -> (tempfile::TempDir, Sources) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("an-order.yaml");
        std::fs::write(&path, DOCUMENT).unwrap();
        let sources = Sources::load(&[path]);
        (dir, sources)
    }

    #[test]
    fn an_assertion_is_located_by_its_path() {
        let (_dir, sources) = loaded();

        let found = sources
            .locate("an-order", "expect.stdout.contains[0]")
            .expect("the case was loaded");

        assert_eq!(
            found.line, 8,
            "the walk follows the path into the document, so the reference lands on the element \
             rather than on the block above it"
        );
        assert!(
            found.reference().ends_with("an-order.yaml:8"),
            "and `path:line` is what a terminal turns into a link: {}",
            found.reference()
        );
    }

    #[test]
    fn a_case_that_was_never_loaded_is_located_nowhere_rather_than_at_line_one() {
        let (_dir, sources) = loaded();

        assert!(
            sources
                .locate("some-other-case", "expect.exit_code")
                .is_none(),
            "a confident `:1` would send the reader to the wrong place, which is worse than \
             sending them nowhere. A case can fail before its document loads at all"
        );
    }

    #[test]
    fn a_document_that_will_not_load_does_not_silence_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.yaml");
        let broken = dir.path().join("broken.yaml");
        std::fs::write(&good, DOCUMENT).unwrap();
        std::fs::write(&broken, "name: [this is not a name\n").unwrap();

        let sources = Sources::load(&[broken, good]);

        assert!(
            sources.locate("an-order", "expect.exit_code").is_some(),
            "the broken case fails for its own reasons and says so; refusing to locate the rest \
             would let one document take the whole report with it"
        );
    }
}
