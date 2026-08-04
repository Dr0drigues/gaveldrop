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

    /// Claims and returns this call's rank for `key`, 1-indexed.
    ///
    /// **One rank goes to exactly one caller, however many are asking at once.** The rank used to be a
    /// number in a file, read then rewritten — a read-modify-write with nothing serialising it. Measured
    /// before this changed: forty concurrent calls produced **five** distinct ranks, eighteen of them
    /// rank 1; thirty separate processes produced twenty-six. Since `call:` is a matching criterion, a
    /// scenario saying "the first call fails and the second succeeds" answered eighteen first calls, and
    /// the journal's rank was wrong wherever it happened.
    ///
    /// So a rank is not counted, it is **claimed**: the caller creates `<key>.<rank>` with `O_CREAT |
    /// O_EXCL` and walks up until one succeeds. That is a single atomic syscall per attempt, which the
    /// kernel decides — no lock to hold, nothing to leave stale if the fake is killed mid-call, and it
    /// works across processes because that is the only place the fake ever has state.
    pub fn next(&self, key: &str) -> Result<u32, CounterError> {
        std::fs::create_dir_all(&self.dir).map_err(|source| CounterError::Io {
            path: self.dir.clone(),
            source,
        })?;

        let stem = file_name_for(key);

        // Where to start looking, and it has to be **verified rather than trusted**. The first version
        // of this trusted it on the grounds that a hint is only written after a rank was won, so it
        // could never point too high. That was wrong, and the continuous-integration run on the other
        // platform is what said so: `std::fs::write` truncates and then writes, so two of them
        // interleaving at offset zero leave a value neither wrote — `"5"` landing over `"37"` reads back
        // as `"57"`. Ranks then got skipped: forty callers came away holding 1 to 37, 55, 88 and 89.
        //
        // Two changes. The hint is written through a rename, which is atomic, so nothing fabricates a
        // value any more. And it is only believed if the rank it names is really claimed — because a
        // hint can also survive a run that was killed, and a wrong one must cost an attempt rather than
        // a gap.
        let hint = self.dir.join(format!("{stem}.from"));
        let first = std::fs::read_to_string(&hint)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok())
            .filter(|rank| *rank >= 1 && self.dir.join(format!("{stem}.{rank}")).exists())
            .unwrap_or(1);

        for rank in first..=u32::MAX {
            let claim = self.dir.join(format!("{stem}.{rank}"));
            match std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&claim)
            {
                Ok(_) => {
                    remember(&hint, rank);
                    return Ok(rank);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(CounterError::Io {
                        path: claim,
                        source,
                    });
                }
            }
        }

        // Four billion calls to one key. Saturating rather than erroring, as before: the case is about
        // something else, and a rank that stops climbing is observable where a crash is not.
        Ok(u32::MAX)
    }

    /// The state directory, for callers that want to read it directly.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Records where the next walk may start, atomically or not at all.
///
/// Through a uniquely-named temporary and a rename, because `rename` replaces in one step where
/// `write` truncates and then fills: two concurrent writes at offset zero produce a number neither
/// caller had, and a fabricated hint sends the next walk past ranks nobody holds.
///
/// Every failure is ignored. This is an optimisation — the walk is correct without it — so a hint that
/// could not be written costs attempts and nothing else.
fn remember(hint: &Path, rank: u32) {
    let Some(dir) = hint.parent() else {
        return;
    };

    // The pid keeps two processes apart and the rank keeps two threads of one process apart, since a
    // rank belongs to exactly one caller by the time we are here.
    let staging = dir.join(format!(".from-{}-{rank}", std::process::id()));
    if std::fs::write(&staging, rank.to_string()).is_ok()
        && std::fs::rename(&staging, hint).is_err()
    {
        let _ = std::fs::remove_file(&staging);
    }
}

/// Turns an arbitrary key into a safe file name stem.
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
    let _ = write!(safe, "-{:016x}", fnv1a(key));
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

        // The property, not the file count: a rank is now claimed as one file per call plus an advisory
        // hint, so counting entries would only measure the layout. What must hold is that every name
        // stays a name — nothing that a path separator or a `..` could carry out of the directory.
        let written: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();

        assert!(!written.is_empty(), "something was written");
        for name in &written {
            assert!(
                !name.contains('/') && !name.contains(".."),
                "nothing may be able to leave the state directory: {name:?} among {written:?}"
            );
            assert!(
                dir.path().join(name).parent() == Some(dir.path()),
                "and every file resolves inside it: {name:?}"
            );
        }
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

    /// Corrupt state must not kill the fake, and must not hand out a rank twice.
    ///
    /// This replaces a test that asserted a garbled counter file restarts from one. That behaviour was a
    /// property of storing the rank as a number; now the only thing corruptible is the advisory hint,
    /// and what matters is stronger — a hint of nonsense must still produce a rank nobody holds.
    #[test]
    fn a_corrupt_hint_still_yields_a_rank_nobody_holds() {
        let dir = tempfile::tempdir().unwrap();
        let counter = Counter::new(dir.path());
        assert_eq!(counter.next("git").unwrap(), 1);
        assert_eq!(counter.next("git").unwrap(), 2);

        for hint in std::fs::read_dir(dir.path()).unwrap() {
            let path = hint.unwrap().path();
            if path.extension().is_some_and(|end| end == "from") {
                std::fs::write(&path, "not a number").unwrap();
            }
        }

        assert_eq!(
            counter.next("git").unwrap(),
            3,
            "a hint of nonsense sends the walk back to one, where it finds 1 and 2 already claimed and \
             takes the first free rank — never a rank someone else is holding"
        );
    }

    /// A hint naming a rank nobody holds is ignored rather than followed.
    ///
    /// The defect the continuous-integration run found in the first version of this: the hint was
    /// trusted, and two non-atomic writes to it fabricated a number neither caller had. Forty callers
    /// came away holding 1 to 37, 55, 88 and 89 — distinct, so nothing was double-booked, but ranks
    /// were skipped and a scenario's `call: 38` would never have matched. The write is a rename now, and
    /// the hint is checked against reality besides, because one can also survive a run that was killed.
    #[test]
    fn a_hint_pointing_past_reality_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let counter = Counter::new(dir.path());
        assert_eq!(counter.next("git").unwrap(), 1);

        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|end| end == "from") {
                std::fs::write(&path, "88").unwrap();
            }
        }

        assert_eq!(
            counter.next("git").unwrap(),
            2,
            "rank 88 is claimed by nobody, so the hint is a fabrication and the walk starts over — \
             following it would leave 2 to 87 free for ever and break a `call:` that names one of them"
        );
    }

    /// One rank to one caller, whatever the concurrency.
    ///
    /// **The defect this whole mechanism was rewritten for.** The rank was a number in a file, read
    /// then rewritten, with nothing serialising it: forty concurrent calls produced five distinct ranks
    /// and eighteen of them were rank 1. Since `call:` is a matching criterion, a scenario saying "the
    /// first call fails and the second succeeds" answered eighteen first calls — silently, and the
    /// journal recorded the wrong rank wherever it happened.
    ///
    /// The suite had no concurrent test at all, which is why five rounds of adversarial stress-testing
    /// on the runtime never touched it: every existing test called `next` one after another.
    #[test]
    fn no_two_callers_get_the_same_rank() {
        const CALLERS: usize = 40;

        let dir = tempfile::tempdir().unwrap();
        let counter = std::sync::Arc::new(Counter::new(dir.path()));

        let claimed: Vec<u32> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..CALLERS)
                .map(|_| {
                    let counter = std::sync::Arc::clone(&counter);
                    scope.spawn(move || counter.next("git").unwrap())
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        let distinct: std::collections::BTreeSet<u32> = claimed.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            CALLERS,
            "every caller must hold a rank of its own; got {claimed:?}"
        );
        assert_eq!(
            distinct.iter().copied().collect::<Vec<_>>(),
            (1..=CALLERS as u32).collect::<Vec<_>>(),
            "and the ranks must be the run of integers from one, with no gap. A gap means the hint \
             that says where to start walking named a rank nobody holds — which is what a \
             non-atomic write to it produces"
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
