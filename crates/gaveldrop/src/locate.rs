//! Finding the fake binary, which is not the same problem as finding a library.
//!
//! Every other dependency of a suite is a crate: cargo resolves it and the compiler links it. The
//! fake is an **executable**, because a subject finds a faked tool by name on `PATH` and only an
//! executable is findable that way. That is an invariant, and this module is the price of it —
//! cargo has no notion of "this binary needs that other binary at runtime", so somebody has to
//! look.
//!
//! It matters beyond the command line: a project running its suite through
//! `runner::run_all_with` passes a fake binary too, and had no way to find one but to hardcode a
//! path.

use std::path::{Path, PathBuf};

/// The name the subject's `PATH` entries are symlinked to.
pub const FAKE: &str = "gaveldrop-fake";

/// Where a fake binary was found, and how — the "how" is what makes a failure diagnosable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// Beside the running executable. The usual case: a cargo build, or an unpacked release.
    Beside(PathBuf),
    /// Somewhere on `PATH`. What a package manager that splits binaries across directories gives.
    OnPath(PathBuf),
}

impl Found {
    /// The path itself.
    pub fn path(&self) -> &Path {
        match self {
            Self::Beside(path) | Self::OnPath(path) => path,
        }
    }
}

/// Looks beside `exe_dir` first, then along `search_path`.
///
/// Beside first, deliberately. A checkout being worked on has its own freshly built fake in
/// `target/debug`, and an older one installed in `~/.cargo/bin` — taking the installed one would
/// mean testing a change against the fake from before it, which fails in a way nobody would guess.
///
/// Taking the search path as an argument rather than reading the environment keeps this testable
/// in parallel with other tests, and it is the same reason `iso::without` is shaped that way.
pub fn fake(exe_dir: &Path, search_path: Option<&std::ffi::OsStr>) -> Option<Found> {
    let beside = exe_dir.join(FAKE);
    if is_program(&beside) {
        return Some(Found::Beside(beside));
    }

    let path = search_path?;
    std::env::split_paths(path)
        .map(|dir| dir.join(FAKE))
        .find(|candidate| is_program(candidate))
        .map(Found::OnPath)
}

/// What to tell someone who has no fake anywhere, which depends on how they got here.
///
/// Two audiences with nothing in common. Someone who ran `cargo install gaveldrop-cli` has a
/// working `gaveldrop` and no fake, because cargo installs the binaries of the crate you named and
/// not those of its dependencies — there is no way for the cli crate to ask for it. Someone in a
/// checkout has simply not built the workspace. Guessing wrong wastes their time, so the message
/// carries both and says which is which.
pub fn advice(exe_dir: &Path) -> String {
    format!(
        "no `{FAKE}` beside {} nor anywhere on PATH, and it is what shadows the dependencies a \
         case fakes.\n  installed with cargo:  cargo install {FAKE} --locked  (the cli crate \
         cannot pull in another crate's binary, so it takes two commands)\n  in a checkout:         \
         cargo build --workspace  (the fake belongs to its own crate and `-p gaveldrop-cli` does \
         not build it)",
        exe_dir.display()
    )
}

/// Beside the **running** executable, then along the current `PATH`.
///
/// The convenience over [`fake`] for callers that have no reason to want anything else.
pub fn fake_for_current_exe() -> Option<Found> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    fake(&dir, std::env::var_os("PATH").as_deref())
}

/// True when the path is a file somebody could execute.
///
/// The executable bit is checked rather than only existence: a directory called `gaveldrop-fake`,
/// or a leftover text file, would otherwise be reported as the fake and fail later with a
/// diagnostic about something else entirely.
fn is_program(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn a_fake_in(dir: &Path) -> PathBuf {
        let path = dir.join(FAKE);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn a_fake_beside_the_executable_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let expected = a_fake_in(dir.path());

        assert_eq!(
            fake(dir.path(), None),
            Some(Found::Beside(expected)),
            "the usual case, and the one that must not need a PATH at all"
        );
    }

    #[test]
    fn a_fake_on_the_path_is_found_when_there_is_none_beside() {
        let empty = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let expected = a_fake_in(elsewhere.path());
        let path = std::env::join_paths([elsewhere.path()]).unwrap();

        assert_eq!(
            fake(empty.path(), Some(&path)),
            Some(Found::OnPath(expected)),
            "`cargo install gaveldrop-cli` installs one binary and no fake. Looking along PATH is \
             what makes the second install work wherever it landed, rather than only when the two \
             happen to share a directory"
        );
    }

    #[test]
    fn the_one_beside_wins_over_the_one_on_the_path() {
        let here = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let beside = a_fake_in(here.path());
        a_fake_in(installed.path());
        let path = std::env::join_paths([installed.path()]).unwrap();

        assert_eq!(
            fake(here.path(), Some(&path)),
            Some(Found::Beside(beside)),
            "a checkout being worked on has a freshly built fake beside the binary and an older \
             installed one on PATH. Preferring the installed one would test a change against the \
             fake from before it — a failure nobody would guess at"
        );
    }

    #[test]
    fn a_file_that_is_not_executable_is_not_the_fake() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FAKE), "not a program").unwrap();

        assert_eq!(
            fake(dir.path(), None),
            None,
            "reporting it would trade a clear message for a failure about something else, once \
             the subject tried to run it"
        );
    }

    #[test]
    fn a_directory_of_that_name_is_not_the_fake() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(FAKE)).unwrap();

        assert_eq!(fake(dir.path(), None), None);
    }

    #[test]
    fn nothing_anywhere_is_reported_as_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([other.path()]).unwrap();

        assert_eq!(fake(dir.path(), Some(&path)), None);
    }

    #[test]
    fn the_advice_covers_both_ways_of_getting_here() {
        let text = advice(Path::new("/somewhere/bin"));

        assert!(
            text.contains("cargo install gaveldrop-fake"),
            "someone who installed the cli has a working `gaveldrop` and no fake, because cargo \
             installs the binaries of the crate you name and not its dependencies': {text}"
        );
        assert!(
            text.contains("cargo build --workspace"),
            "and someone in a checkout has simply not built it — `-p gaveldrop-cli` does not, \
             which has bitten here before: {text}"
        );
        assert!(
            text.contains("/somewhere/bin"),
            "naming where it looked is what stops the reader wondering whether it looked in the \
             right place: {text}"
        );
    }
}
