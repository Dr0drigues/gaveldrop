//! The rank of a call, persisted from one process to the next.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Call counter, one file per key inside a directory.
///
/// Every intercepted call is a **different process**, so the counter cannot live in
/// memory. It lives in a file the core prepared, which the fake reads then rewrites
/// each time.
///
/// The **key** is supplied by the caller, not inferred. By default it is the name of
/// the faked binary; a project may put something else there — an agent identifier
/// extracted from standard input, for instance. Two semantics, one mechanism, and the
/// choice stays visible in the caller's code rather than hidden in the engine.
pub struct Counter {
    dir: PathBuf,
}

/// What can go wrong with the counter.
#[derive(Debug, thiserror::Error)]
pub enum CounterError {
    /// The core did not set the state directory variable before invoking the fake.
    #[error("environment variable {0} is missing: the core must set it before invoking the fake")]
    MissingEnv(&'static str),
    /// The state directory or one of its files could not be written.
    #[error("writing the counter under {path}: {source}")]
    Io {
        /// The path that could not be written.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
}

impl Counter {
    /// A counter living in `dir`. The directory is created on the first call.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The counter designated by [`crate::env::STATE`].
    pub fn from_env() -> Result<Self, CounterError> {
        let dir = std::env::var_os(crate::env::STATE)
            .ok_or(CounterError::MissingEnv(crate::env::STATE))?;
        Ok(Self::new(PathBuf::from(dir)))
    }

    /// Increments and returns this call's rank for `key`, 1-indexed.
    ///
    /// A file that is missing, empty or corrupt counts as zero, so the rank restarts
    /// from one. An unreadable counter must not kill the fake: the case is about
    /// something else, and restarting from one is observable behaviour rather than a
    /// crash.
    pub fn next(&self, key: &str) -> Result<u32, CounterError> {
        std::fs::create_dir_all(&self.dir).map_err(|source| CounterError::Io {
            path: self.dir.clone(),
            source,
        })?;

        let path = self.dir.join(file_name_for(key));

        let current: u32 = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0);
        let next = current.saturating_add(1);

        std::fs::write(&path, next.to_string())
            .map_err(|source| CounterError::Io { path, source })?;

        Ok(next)
    }

    /// The state directory, for callers that want to read it directly.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Turns an arbitrary key into a safe file name.
///
/// Two requirements, and the second is the one that gets forgotten: the name must not
/// be able to escape the state directory (`../..`), and **two distinct keys must not
/// produce the same name**. Simply replacing awkward characters fails the second
/// point, since `a/b` and `a b` would both become `a_b`. So the name is suffixed with
/// a fingerprint of the original key.
///
/// The readable part is truncated to 180 characters before the fingerprint: some file
/// systems cap names at 255 bytes, and a key can be long — a URL, for instance.
fn file_name_for(key: &str) -> String {
    let mut safe = String::with_capacity(key.len() + 24);
    for character in key.chars() {
        if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
            safe.push(character);
        } else {
            safe.push('_');
        }
    }
    safe.truncate(180);
    let _ = write!(safe, "-{:016x}.count", fnv1a(key));
    safe
}

/// FNV-1a, 64 bits. Chosen because it fits in ten lines and needs no dependency: this
/// discriminates keys, it protects nothing.
fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_call_is_one() {
        let dir = tempfile::tempdir().unwrap();
        let counter = Counter::new(dir.path());
        assert_eq!(counter.next("git").unwrap(), 1);
    }

    #[test]
    fn the_counter_persists_between_calls() {
        let dir = tempfile::tempdir().unwrap();
        let counter = Counter::new(dir.path());
        assert_eq!(counter.next("git").unwrap(), 1);
        assert_eq!(counter.next("git").unwrap(), 2);

        let other = Counter::new(dir.path());
        assert_eq!(
            other.next("git").unwrap(),
            3,
            "another Counter over the same directory must continue the series: every \
             intercepted call is a different process"
        );
    }

    #[test]
    fn each_key_has_its_own_counter() {
        let dir = tempfile::tempdir().unwrap();
        let counter = Counter::new(dir.path());
        assert_eq!(counter.next("git").unwrap(), 1);
        assert_eq!(counter.next("kubectl").unwrap(), 1);
        assert_eq!(counter.next("git").unwrap(), 2);
    }

    #[test]
    fn a_key_with_path_characters_cannot_escape_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let counter = Counter::new(dir.path());
        assert_eq!(counter.next("../../escaped").unwrap(), 1);
        assert_eq!(counter.next("POST /orders").unwrap(), 1);

        let written: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            written.len(),
            2,
            "nothing may be written outside the state directory; files: {written:?}"
        );
        assert!(written.iter().all(|name| !name.contains('/')));
    }

    #[test]
    fn two_different_keys_do_not_collide_after_sanitising() {
        let dir = tempfile::tempdir().unwrap();
        let counter = Counter::new(dir.path());
        assert_eq!(counter.next("a/b").unwrap(), 1);
        assert_eq!(
            counter.next("a b").unwrap(),
            1,
            "`a/b` and `a b` would both sanitise to `a_b` if the file name were the \
             only discriminator"
        );
    }

    #[test]
    fn an_unreadable_counter_file_restarts_from_one() {
        let dir = tempfile::tempdir().unwrap();
        let counter = Counter::new(dir.path());
        counter.next("git").unwrap();

        let path = std::fs::read_dir(dir.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::write(&path, "not a number").unwrap();

        assert_eq!(
            counter.next("git").unwrap(),
            1,
            "a counter overwritten with anything must restart from one, not panic"
        );
    }

    #[test]
    fn the_directory_is_created_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("not/there/yet");
        let counter = Counter::new(&missing);
        assert_eq!(counter.next("git").unwrap(), 1);
        assert!(missing.is_dir());
    }
}
