//! The tree snapshot, and the difference the subject made to it.
//!
//! The observation takes **everything** that changed — the isolated directory is tiny, so
//! walking it costs nothing — and the assertion names paths. There is therefore no
//! trade-off between "full diff" and "watched list": they are two different layers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Largest file whose content is captured. Beyond it, `content` is `None`.
///
/// A cap rather than no cap: without one, a subject that writes a gigabyte would put the
/// report's memory at its own mercy. 256 KiB is far above any configuration file and far
/// below anything that hurts.
pub const MAX_CAPTURED_BYTES: usize = 256 * 1024;

/// What happened to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChange {
    /// It did not exist before the run.
    Created,
    /// It existed and its content differs.
    Modified,
    /// It existed and no longer does.
    Removed,
}

/// One file the subject touched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEffect {
    /// Path relative to the isolated root. Relative because an absolute temporary path
    /// would make every failure message unreadable and every case unportable.
    pub path: PathBuf,
    /// Whether it was created, modified or removed.
    pub change: FileChange,
    /// Size in bytes after the run. Zero for a removed file.
    pub size: u64,
    /// The content, when it is UTF-8 and under [`MAX_CAPTURED_BYTES`].
    ///
    /// `None` means "not assertable", never "empty". An assertion against such a file must
    /// say so rather than pass.
    pub content: Option<String>,
}

/// The state of a tree at a point in time.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    files: BTreeMap<PathBuf, Fingerprint>,
}

/// Enough of a file to tell whether it changed.
///
/// Size **and** content hash. Size alone would miss an in-place edit that keeps the length,
/// which is exactly the shape of a template rendering the wrong value into the same slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    size: u64,
    hash: u64,
}

impl Snapshot {
    /// Records the state of everything under `root`, skipping what belongs to the engine.
    pub fn take(root: &Path) -> Self {
        let mut files = BTreeMap::new();
        collect(root, root, &mut files);
        Self { files }
    }

    /// What changed under `root` since this snapshot was taken.
    pub fn changes_since(&self, root: &Path) -> Vec<FileEffect> {
        let after = Self::take(root);
        let mut effects = Vec::new();

        for (path, fingerprint) in &after.files {
            let change = match self.files.get(path) {
                None => FileChange::Created,
                Some(before) if before != fingerprint => FileChange::Modified,
                Some(_) => continue,
            };
            effects.push(FileEffect {
                path: path.clone(),
                change,
                size: fingerprint.size,
                content: read_capturable(&root.join(path), fingerprint.size),
            });
        }

        for path in self.files.keys() {
            if !after.files.contains_key(path) {
                effects.push(FileEffect {
                    path: path.clone(),
                    change: FileChange::Removed,
                    size: 0,
                    content: None,
                });
            }
        }

        effects.sort_by(|left, right| left.path.cmp(&right.path));
        effects
    }
}

/// Walks `dir`, recording every regular file relative to `root`.
fn collect(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Fingerprint>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        if is_engine_bookkeeping(relative) {
            continue;
        }

        match entry.file_type() {
            Ok(kind) if kind.is_dir() => collect(root, &path, files),
            Ok(kind) if kind.is_file() => {
                if let Some(fingerprint) = fingerprint_of(&path) {
                    files.insert(relative.to_path_buf(), fingerprint);
                }
            }
            _ => {}
        }
    }
}

/// True for the paths gaveldrop itself writes into the isolated root.
///
/// The scenario, the journal, the call counter and the directory of symlinks are ours.
/// Reporting them would drown every case in noise the case cannot control, and a `files`
/// assertion would have to enumerate our implementation details to stay quiet.
fn is_engine_bookkeeping(relative: &Path) -> bool {
    const OURS: &[&str] = &["scenario.yaml", "journal.jsonl", "state", "bin"];

    relative
        .components()
        .next()
        .and_then(|first| first.as_os_str().to_str())
        .is_some_and(|first| OURS.contains(&first))
}

/// Size and content hash, or `None` when the file cannot be read.
fn fingerprint_of(path: &Path) -> Option<Fingerprint> {
    let bytes = std::fs::read(path).ok()?;
    Some(Fingerprint {
        size: bytes.len() as u64,
        hash: fnv1a(&bytes),
    })
}

/// The file's content when it is assertable: UTF-8 and under the cap.
fn read_capturable(path: &Path, size: u64) -> Option<String> {
    if size > MAX_CAPTURED_BYTES as u64 {
        return None;
    }
    std::fs::read(path)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

/// FNV-1a, 64 bits. Enough to tell two contents apart; it protects nothing.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, relative: &str, body: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    fn effects(root: &Path, before: &Snapshot) -> BTreeMap<String, FileEffect> {
        before
            .changes_since(root)
            .into_iter()
            .map(|effect| (effect.path.to_string_lossy().into_owned(), effect))
            .collect()
    }

    #[test]
    fn a_created_file_is_reported_with_its_content() {
        let dir = tempfile::tempdir().unwrap();
        let before = Snapshot::take(dir.path());
        write(dir.path(), ".config/k9s/plugins.yaml", "log-view-pod\n");

        let changed = effects(dir.path(), &before);
        let effect = &changed[".config/k9s/plugins.yaml"];

        assert_eq!(effect.change, FileChange::Created);
        assert_eq!(effect.content.as_deref(), Some("log-view-pod\n"));
        assert_eq!(
            effect.path,
            PathBuf::from(".config/k9s/plugins.yaml"),
            "paths are relative to the isolated root: an absolute temporary path would \
             make every failure message unreadable and every case unportable"
        );
    }

    #[test]
    fn an_untouched_file_is_not_reported() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "kept.txt", "same");
        let before = Snapshot::take(dir.path());

        assert!(
            before.changes_since(dir.path()).is_empty(),
            "the diff must report what the subject did, not what setup left behind"
        );
    }

    #[test]
    fn a_modified_file_is_reported_as_modified() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "config.yaml", "before");
        let before = Snapshot::take(dir.path());
        write(dir.path(), "config.yaml", "after");

        let changed = effects(dir.path(), &before);
        assert_eq!(changed["config.yaml"].change, FileChange::Modified);
        assert_eq!(changed["config.yaml"].content.as_deref(), Some("after"));
    }

    #[test]
    fn a_removed_file_is_reported_with_no_content() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "doomed.txt", "bye");
        let before = Snapshot::take(dir.path());
        std::fs::remove_file(dir.path().join("doomed.txt")).unwrap();

        let changed = effects(dir.path(), &before);
        assert_eq!(changed["doomed.txt"].change, FileChange::Removed);
        assert!(changed["doomed.txt"].content.is_none());
    }

    #[test]
    fn a_file_that_only_changed_content_at_equal_size_is_still_detected() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "same-size.txt", "aaaa");
        let before = Snapshot::take(dir.path());
        write(dir.path(), "same-size.txt", "bbbb");

        let changed = effects(dir.path(), &before);
        assert_eq!(
            changed["same-size.txt"].change,
            FileChange::Modified,
            "comparing sizes alone would miss an in-place edit, which is exactly the shape \
             of a bad template render"
        );
    }

    #[test]
    fn binary_and_oversized_files_are_reported_without_content() {
        let dir = tempfile::tempdir().unwrap();
        let before = Snapshot::take(dir.path());
        std::fs::write(dir.path().join("blob.bin"), [0xff, 0xfe, 0x00]).unwrap();
        std::fs::write(
            dir.path().join("huge.txt"),
            "x".repeat(MAX_CAPTURED_BYTES + 1),
        )
        .unwrap();

        let changed = effects(dir.path(), &before);
        assert!(
            changed["blob.bin"].content.is_none(),
            "non-UTF-8 content has no business in a text assertion"
        );
        assert!(
            changed["huge.txt"].content.is_none(),
            "capturing an arbitrarily large file would put the report's memory at the mercy \
             of the subject under test"
        );
        assert!(changed["huge.txt"].size > MAX_CAPTURED_BYTES as u64);
    }

    #[test]
    fn the_engine_s_own_bookkeeping_is_never_reported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("state")).unwrap();
        let before = Snapshot::take(dir.path());

        std::fs::write(dir.path().join("journal.jsonl"), "{}\n").unwrap();
        std::fs::write(dir.path().join("scenario.yaml"), "rules: []\n").unwrap();
        std::fs::write(dir.path().join("state/git.count"), "1").unwrap();
        std::fs::create_dir_all(dir.path().join("bin")).unwrap();
        std::fs::write(dir.path().join("bin/git"), "").unwrap();
        write(dir.path(), "real.txt", "the subject's own work");

        let changed = effects(dir.path(), &before);
        assert_eq!(
            changed.keys().collect::<Vec<_>>(),
            vec!["real.txt"],
            "the scenario, the journal, the counter and the symlink directory are ours. \
             Reporting them would drown every case in noise the case cannot control"
        );
    }
}
