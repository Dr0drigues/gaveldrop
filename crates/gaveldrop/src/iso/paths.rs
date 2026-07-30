//! Resolving the paths a case writes.
//!
//! This is the format's **only** interpolation, and it exists out of necessity: the isolated
//! root is a temporary directory with an unpredictable name, so without substitution a case
//! could not name any file at all.
//!
//! It is bounded to match. A case may use exactly the variables isolation itself defines —
//! `$HOME`, the `XDG_*` family — plus a leading `~`. No shell expansion, and nothing from
//! the environment of whoever runs the tests.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// A path a case could not resolve.
#[derive(Debug, thiserror::Error)]
pub enum PathError {
    /// The path names a variable isolation does not define.
    #[error(
        "unknown variable ${name} in path {pattern:?}. A case may only use what isolation \
         defines: {available}"
    )]
    UnknownVariable {
        /// The variable nobody defines.
        name: String,
        /// The path it appeared in.
        pattern: String,
        /// The variables that would have worked, comma-separated.
        available: String,
    },
    /// The path resolves somewhere isolation does not reach.
    #[error(
        "path {pattern:?} resolves to {resolved}, outside the isolated root. Nothing is \
         observed out there, so no assertion about it could ever hold"
    )]
    OutsideRoot {
        /// The path as the case wrote it.
        pattern: String,
        /// Where it landed.
        resolved: String,
    },
}

/// Resolves `pattern` to a path **relative to the isolated root**.
///
/// Relative because that is what the tree diff reports, and comparing an absolute
/// substitution against a relative observation would silently never match. One function owns
/// the whole resolution rather than leaving each caller to strip a prefix.
///
/// An unknown variable is an **error**, never left literal. `$TYPO` surviving into the
/// resolved path would make an `absent` assertion trivially true — a green case asserting
/// nothing, which is the worst outcome available.
pub fn substitute(pattern: &str, defined: &BTreeMap<String, String>) -> Result<PathBuf, PathError> {
    let root = defined.get("HOME").cloned().unwrap_or_default();
    let out = expand(pattern, defined)?;
    relative_to_root(&out, &root, pattern)
}

/// Substitutes what isolation defines and leaves everything else alone.
///
/// For a **command line**, not a path. `serve: ["python3", "$GAVELDROP_PROJECT/app/server.py"]` has to
/// resolve, because the subject runs in the isolated directory where the project does not exist.
///
/// Unknown names are left literal, which is the opposite of [`substitute`] and correct here: a
/// command is very often a shell script, and `${MYVAR-default}` is that shell's syntax to read. Being
/// strict would refuse `printf %s "${HOME-none}"` — a legitimate command — for using a construct that
/// was never ours to interpret. In a path a stray `$TYPO` has to be an error, because it would make an
/// `absent` assertion trivially true; in a command it is a word the shell will deal with.
pub fn expand_known(pattern: &str, defined: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut rest = pattern;

    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let (name, remainder, braced) = split_name(after);

        match defined.get(name) {
            Some(value) => out.push_str(value),
            None => {
                out.push('$');
                if braced {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                } else {
                    out.push_str(name);
                }
            }
        }
        rest = remainder;
    }

    out.push_str(rest);
    out
}

/// The variable name after a `$`, what follows it, and whether it was braced.
fn split_name(after: &str) -> (&str, &str, bool) {
    match after.strip_prefix('{') {
        Some(braced) => match braced.find('}') {
            Some(end) => (&braced[..end], &braced[end + 1..], true),
            None => (braced, "", true),
        },
        None => {
            let end = after
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(after.len());
            (&after[..end], &after[end..], false)
        }
    }
}

/// Expands the variables in `pattern` without deciding anything about paths.
///
/// The same bounded, **strict** interpolation as [`substitute`], stopping before the root check.
///
/// This is what an environment variable declared by a case needs, and neither of the other two
/// would do. [`substitute`] confines its result under the isolated home, but
/// `ZANVIL_DIR: "$GAVELDROP_PROJECT"` legitimately points at the repository. [`expand_known`] is
/// lenient because a command line is read by a shell whose syntax is not ours — an environment
/// value is handed to `Command::env` and no shell ever sees it, so a stray `$TYPO` is a mistake
/// rather than a construct to preserve, and passing it through would set the variable to something
/// silently wrong.
pub fn expand(pattern: &str, defined: &BTreeMap<String, String>) -> Result<String, PathError> {
    let expanded = match pattern.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", lookup("HOME", pattern, defined)?),
        None => pattern.to_string(),
    };

    let mut out = String::with_capacity(expanded.len());
    let mut rest = expanded.as_str();

    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];

        let (name, remainder) = match after.strip_prefix('{') {
            Some(braced) => match braced.find('}') {
                Some(end) => (&braced[..end], &braced[end + 1..]),
                None => (braced, ""),
            },
            None => {
                let end = after
                    .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .unwrap_or(after.len());
                (&after[..end], &after[end..])
            }
        };

        out.push_str(&lookup(name, pattern, defined)?);
        rest = remainder;
    }

    out.push_str(rest);
    Ok(out)
}

/// Strips the isolated root, so the result lines up with what the tree diff reports.
///
/// A path that is absolute and *not* under the root is refused rather than compared: nothing
/// is observed out there, so an assertion about it could never hold, and failing with that
/// sentence beats failing with "not written".
fn relative_to_root(resolved: &str, root: &str, pattern: &str) -> Result<PathBuf, PathError> {
    if !resolved.starts_with('/') {
        return Ok(PathBuf::from(resolved));
    }

    if !root.is_empty()
        && let Some(inside) = resolved.strip_prefix(root)
    {
        return Ok(PathBuf::from(inside.trim_start_matches('/')));
    }

    Err(PathError::OutsideRoot {
        pattern: pattern.to_string(),
        resolved: resolved.to_string(),
    })
}

/// One variable, or an error naming what was available instead.
fn lookup(
    name: &str,
    pattern: &str,
    defined: &BTreeMap<String, String>,
) -> Result<String, PathError> {
    defined
        .get(name)
        .cloned()
        .ok_or_else(|| PathError::UnknownVariable {
            name: name.to_string(),
            pattern: pattern.to_string(),
            available: defined.keys().cloned().collect::<Vec<_>>().join(", "),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defined() -> BTreeMap<String, String> {
        [
            ("HOME".to_string(), "/iso".to_string()),
            ("XDG_CONFIG_HOME".to_string(), "/iso/.config".to_string()),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn a_command_keeps_a_shell_default_it_does_not_own() {
        let defined = BTreeMap::from([("HOME".to_string(), "/iso".to_string())]);

        assert_eq!(
            expand_known("printf %s \"${MYVAR-none}\"", &defined),
            "printf %s \"${MYVAR-none}\"",
            "`${{MYVAR-none}}` is the shell's syntax for a default. Refusing it would reject a \
             legitimate command for using a construct that was never ours to interpret"
        );
    }

    #[test]
    fn a_command_substitutes_what_isolation_defines() {
        let defined = BTreeMap::from([("GAVELDROP_PROJECT".to_string(), "/repo".to_string())]);

        assert_eq!(
            expand_known("$GAVELDROP_PROJECT/app/server.py", &defined),
            "/repo/app/server.py",
            "a service is a file of the project, and the subject runs where the project is not"
        );
    }

    #[test]
    fn a_command_substitutes_a_braced_name_too() {
        let defined = BTreeMap::from([("GAVELDROP_PORT".to_string(), "8080".to_string())]);

        assert_eq!(
            expand_known("http://127.0.0.1:${GAVELDROP_PORT}/health", &defined),
            "http://127.0.0.1:8080/health"
        );
    }

    #[test]
    fn home_and_tilde_both_resolve_relative_to_the_isolated_root() {
        assert_eq!(
            substitute("$HOME/Library/k9s/plugins.yaml", &defined()).unwrap(),
            PathBuf::from("Library/k9s/plugins.yaml"),
            "the result must line up with what the tree diff reports, which is relative: an \
             absolute substitution would silently never match"
        );
        assert_eq!(
            substitute("~/.zshrc", &defined()).unwrap(),
            PathBuf::from(".zshrc")
        );
    }

    #[test]
    fn braced_and_bare_forms_are_both_accepted() {
        assert_eq!(
            substitute("${XDG_CONFIG_HOME}/foo", &defined()).unwrap(),
            PathBuf::from(".config/foo")
        );
        assert_eq!(
            substitute("$XDG_CONFIG_HOME/foo", &defined()).unwrap(),
            PathBuf::from(".config/foo")
        );
    }

    #[test]
    fn an_absolute_path_outside_the_root_is_refused_rather_than_never_matched() {
        let error = substitute("/etc/passwd", &defined()).unwrap_err();

        assert!(
            error.to_string().contains("outside the isolated root"),
            "nothing is observed out there, so failing with that sentence beats failing \
             with `not written`: {error}"
        );
    }

    #[test]
    fn a_relative_path_stays_relative_to_the_isolated_root() {
        assert_eq!(
            substitute("out/report.json", &defined()).unwrap(),
            PathBuf::from("out/report.json")
        );
    }

    #[test]
    fn an_unknown_variable_is_an_error_rather_than_a_literal() {
        let error = substitute("$TYPO/plugins.yaml", &defined()).unwrap_err();

        assert!(
            error.to_string().contains("TYPO"),
            "leaving an unknown variable literal would make an `absent` assertion trivially \
             true — a green case asserting nothing. Name it instead: {error}"
        );
        assert!(
            error.to_string().contains("HOME"),
            "and say which variables are available: {error}"
        );
    }

    #[test]
    fn a_variable_from_the_developers_environment_is_not_available() {
        let error = substitute("$PATH/nope", &defined()).unwrap_err();

        assert!(
            error.to_string().contains("PATH"),
            "only what isolation itself defines may be substituted, or a case would depend \
             on whoever runs it: {error}"
        );
    }
}
