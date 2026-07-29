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
    relative_to_root(&out, &root, pattern)
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
