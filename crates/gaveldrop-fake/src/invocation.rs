//! What the fake binary sees of a call.

use std::io::{IsTerminal, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// One intercepted call, normalised.
///
/// This is the only view a rule has of its caller, and it is also what goes into
/// the render hook — hence `Serialize`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Invocation {
    /// The name the fake was invoked under, stripped of its path. This is the name
    /// of the faked binary: the fake is symlinked as `git`, `kubectl` and so on,
    /// and finds out which one it is through its `argv[0]`.
    pub bin: String,
    /// The arguments, without `argv[0]`.
    pub args: Vec<String>,
    /// Standard input, read in full — or empty if nobody asked for it.
    pub stdin: String,
}

impl Invocation {
    /// Builds an invocation from the process environment.
    ///
    /// `read_stdin` must only be true when at least one rule in the scenario uses
    /// `stdin_contains`. Reading standard input unconditionally would make the fake
    /// **block forever** whenever its caller handed it an inherited pipe that never
    /// gets closed — which is the common case for a program launched by a test
    /// harness.
    pub fn from_env(read_stdin: bool) -> Self {
        let stdin = if read_stdin {
            read_stdin_now()
        } else {
            String::new()
        };
        Self::from_argv(std::env::args(), stdin)
    }

    /// Testable variant: the arguments are supplied rather than read.
    pub fn from_argv<I: IntoIterator<Item = String>>(argv: I, stdin: String) -> Self {
        let mut it = argv.into_iter();
        let bin = it
            .next()
            .as_deref()
            .and_then(|a| Path::new(a).file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            bin,
            args: it.collect(),
            stdin,
        }
    }

    /// The arguments rejoined with spaces, for `args_contain`.
    ///
    /// The join is naive on purpose: a criterion reads in a case the way it would
    /// read in a terminal, and nobody writes `args_contain` thinking about shell
    /// quoting.
    pub fn args_joined(&self) -> String {
        self.args.join(" ")
    }
}

/// Reads standard input, unless it is a terminal — in which case there is nothing
/// to read and waiting would be a hang.
///
/// An unreadable input counts as an empty one: the fake must never die for a reason
/// the case did not ask for.
fn read_stdin_now() -> String {
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        return String::new();
    }
    let mut buf = String::new();
    let _ = stdin.read_to_string(&mut buf);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_is_the_basename_of_argv0() {
        let inv = Invocation::from_argv(
            ["/tmp/case-1/bin/kubectl", "get", "pods"]
                .iter()
                .map(|s| (*s).to_string()),
            String::new(),
        );
        assert_eq!(
            inv.bin, "kubectl",
            "the fake discovers which binary it stands in for through its argv[0]"
        );
        assert_eq!(inv.args, vec!["get", "pods"]);
    }

    #[test]
    fn args_joined_reassembles_the_arguments() {
        let inv = Invocation::from_argv(
            ["git", "status", "--porcelain"]
                .iter()
                .map(|s| (*s).to_string()),
            String::new(),
        );
        assert_eq!(inv.args_joined(), "status --porcelain");
    }

    #[test]
    fn empty_argv_yields_an_empty_bin_without_panicking() {
        let inv = Invocation::from_argv(std::iter::empty(), String::new());
        assert_eq!(inv.bin, "");
        assert!(inv.args.is_empty());
    }
}
