//! The call journal: who called what, in what order, how many times.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Invocation;

/// One journal line: an intercepted call and what became of it.
///
/// This is what replaces structured events for a technology that emits none.
/// "`kubectl` was called with `--context aks-blg-dev`, and exactly twice" is as strong
/// an assertion as an event count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    /// Name of the faked binary.
    pub bin: String,
    /// The arguments received, without `argv[0]`.
    pub args: Vec<String>,
    /// The rank of this call for its key.
    pub call: u32,
    /// The key the counter was kept under.
    pub key: String,
    /// True when the **catch-all** answered — so a call the case had not foreseen.
    /// This is the most valuable piece of information in the journal.
    pub catch_all: bool,
    /// True when the call was passed through to the real binary.
    pub passthrough: bool,
    /// The exit code the fake returned.
    pub exit: i32,
}

impl Call {
    /// Builds a journal line from the call and what was done about it.
    pub fn from_invocation(
        inv: &Invocation,
        call: u32,
        key: &str,
        catch_all: bool,
        passthrough: bool,
        exit: i32,
    ) -> Self {
        Self {
            bin: inv.bin.clone(),
            args: inv.args.clone(),
            call,
            key: key.to_string(),
            catch_all,
            passthrough,
            exit,
        }
    }
}

/// The journal: a file of JSON lines, opened in append mode.
///
/// **Append-only, never a pipe or a socket.** Each intercepted call is a separate
/// process, and the subject under test may spawn several in parallel. A file opened
/// `O_APPEND` accepts concurrent writes with no coordination at all, as long as they
/// stay under a pipe's size — which a JSON line of this size guarantees by a wide
/// margin.
pub struct Journal {
    path: PathBuf,
}

/// What can go wrong with the journal.
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    /// The core did not set the journal path variable before invoking the fake.
    #[error("environment variable {0} is missing: the core must set it before invoking the fake")]
    MissingEnv(&'static str),
    /// The journal or its parent directory could not be written.
    #[error("writing the journal at {path}: {source}")]
    Io {
        /// The path that could not be written.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// A journal line could not be serialised.
    #[error("serialising a journal line: {0}")]
    Encode(#[from] serde_json::Error),
}

impl Journal {
    /// A journal at this path. The file is created on the first append.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The journal designated by [`crate::env::JOURNAL`].
    pub fn from_env() -> Result<Self, JournalError> {
        let path = std::env::var_os(crate::env::JOURNAL)
            .ok_or(JournalError::MissingEnv(crate::env::JOURNAL))?;
        Ok(Self::new(PathBuf::from(path)))
    }

    /// Appends a line. The parent directory is created if missing.
    pub fn record(&self, call: &Call) -> Result<(), JournalError> {
        let mut line = serde_json::to_string(call)?;
        line.push('\n');

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| JournalError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| JournalError::Io {
                path: self.path.clone(),
                source,
            })?;

        file.write_all(line.as_bytes())
            .map_err(|source| JournalError::Io {
                path: self.path.clone(),
                source,
            })
    }

    /// Reads the journal back.
    ///
    /// A missing journal counts as an empty one: the subject simply called nobody, and
    /// that is a legitimate observation.
    ///
    /// An unreadable line is **skipped** rather than fatal. Losing one line matters
    /// less than losing the other fifty, and the core will still be able to say the
    /// counts do not add up.
    pub fn read(path: &Path) -> Result<Vec<Call>, JournalError> {
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(JournalError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };

        Ok(contents
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect())
    }

    /// The journal's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(bin: &str, rank: u32) -> Call {
        Call {
            bin: bin.to_string(),
            args: vec!["status".to_string()],
            call: rank,
            key: bin.to_string(),
            catch_all: false,
            passthrough: false,
            exit: 0,
        }
    }

    #[test]
    fn a_journaled_call_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        Journal::new(&path).record(&call("git", 1)).unwrap();

        let read = Journal::read(&path).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].bin, "git");
        assert_eq!(read[0].call, 1);
        assert_eq!(read[0].args, vec!["status"]);
    }

    #[test]
    fn calls_are_appended_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let journal = Journal::new(&path);
        journal.record(&call("git", 1)).unwrap();
        journal.record(&call("kubectl", 1)).unwrap();
        journal.record(&call("git", 2)).unwrap();

        let names: Vec<String> = Journal::read(&path)
            .unwrap()
            .into_iter()
            .map(|entry| entry.bin)
            .collect();
        assert_eq!(names, vec!["git", "kubectl", "git"]);
    }

    #[test]
    fn a_journal_written_by_two_instances_stays_complete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        Journal::new(&path).record(&call("git", 1)).unwrap();
        Journal::new(&path).record(&call("git", 2)).unwrap();

        assert_eq!(
            Journal::read(&path).unwrap().len(),
            2,
            "every intercepted call is a different process: two Journal instances over \
             the same path must add up, not overwrite"
        );
    }

    #[test]
    fn a_missing_journal_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let read = Journal::read(&dir.path().join("never-written.jsonl")).unwrap();
        assert!(
            read.is_empty(),
            "a subject that called nobody is a legitimate observation, not an error"
        );
    }

    #[test]
    fn an_unreadable_line_does_not_lose_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let journal = Journal::new(&path);

        let valid = serde_json::to_string(&call("git", 1)).unwrap();
        std::fs::write(&path, format!("{valid}\nthis is not JSON\n")).unwrap();
        journal.record(&call("git", 2)).unwrap();

        assert_eq!(
            Journal::read(&path).unwrap().len(),
            2,
            "losing one line must not lose the other fifty"
        );
    }

    #[test]
    fn the_catch_all_and_passthrough_are_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let journal = Journal::new(&path);
        journal
            .record(&Call {
                catch_all: true,
                ..call("unexpected", 1)
            })
            .unwrap();
        journal
            .record(&Call {
                passthrough: true,
                ..call("sops", 1)
            })
            .unwrap();

        let read = Journal::read(&path).unwrap();
        assert!(
            read[0].catch_all,
            "an unexpected call must be identifiable in the journal: it is the most \
             valuable thing in there"
        );
        assert!(
            read[1].passthrough,
            "a call passed through to the real binary is journaled all the same"
        );
    }

    #[test]
    fn a_call_is_built_from_an_invocation() {
        let inv = Invocation {
            bin: "kubectl".into(),
            args: vec!["get".into(), "pods".into()],
            stdin: String::new(),
        };
        let entry = Call::from_invocation(&inv, 3, "kubectl", true, false, 127);

        assert_eq!(entry.bin, "kubectl");
        assert_eq!(entry.args, vec!["get", "pods"]);
        assert_eq!(entry.call, 3);
        assert_eq!(entry.key, "kubectl");
        assert!(entry.catch_all);
        assert_eq!(entry.exit, 127);
    }
}
