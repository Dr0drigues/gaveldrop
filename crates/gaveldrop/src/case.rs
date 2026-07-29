//! The case format: one YAML document is one test.
//!
//! The doc comments on these types are not internal notes. They travel through the
//! generated JSON schema all the way to the editor of whoever writes a case, so they
//! are the closest thing this format has to a user manual.

pub mod schema;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gaveldrop_fake::Scenario;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One test: how to invoke the subject, how its dependencies must respond, and what the
/// result must contain.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Case {
    /// Human-readable name. Also the identifier in reports, so make it say what the case
    /// proves rather than what it does.
    pub name: String,
    /// How much this case matters. Reports sort failures by it, and gating thresholds
    /// weigh it.
    pub weight: u32,
    /// A known failure that must not fail the whole suite. Opt in deliberately; it is
    /// never inherited by omission.
    #[serde(default)]
    pub allow_fail: bool,
    /// How to prepare and invoke the subject.
    pub setup: Setup,
    /// How the faked dependencies must respond. Omit it when the subject calls nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fake: Option<Scenario>,
    /// What the run must produce.
    pub expect: Expect,
}

/// How to prepare and invoke the subject.
///
/// The core understands exactly two keys, `run` and `exec`. **Everything else is
/// opaque** and travels untouched into the setup hook, which is what lets a project
/// write its own vocabulary here without the core learning any domain words.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct Setup {
    /// The command line to invoke, argument by argument. No shell is involved, so there
    /// are no quoting rules to remember.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<Vec<String>>,
    /// A project executable that prepares the isolated directory. It receives this whole
    /// block as JSON on its standard input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec: Option<String>,
    /// Every other key, kept verbatim for the setup hook.
    ///
    /// Held as JSON rather than YAML values because that is the shape the hook receives
    /// them in, and because it is the one `schemars` can describe.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// What the run must produce.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// The exit code the subject must return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Assertions on standard output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<TextExpectation>,
    /// Assertions on standard error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<TextExpectation>,
    /// How many times each faked binary must have been called. A count of `0` asserts a
    /// dependency was **not** touched, which is often the interesting half.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calls: Option<BTreeMap<String, usize>>,
    /// Structured events that must appear, in order.
    ///
    /// Matched as a **subsequence**: other events may occur in between, and each entry only
    /// constrains the fields it names. An exact list would break the day the subject gains one
    /// new event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<BTreeMap<String, serde_json::Value>>,
    /// How many events of each type must have been emitted. A count of `0` proves one never
    /// happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_counts: Option<BTreeMap<String, usize>>,
    /// Assertions on the files the subject wrote, by path.
    ///
    /// A path may use the variables isolation defines — `$HOME`, `${XDG_CONFIG_HOME}` and the
    /// rest — or a leading `~`. Anything else is refused rather than left literal, because a
    /// stray `$TYPO` would make an `absent` assertion trivially true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<BTreeMap<String, TextExpectation>>,
}

/// Assertions on a stream of text.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextExpectation {
    /// Substrings that must all appear.
    #[serde(default)]
    pub contains: Vec<String>,
    /// Substrings that must appear nowhere. This is the family that catches an
    /// unresolved variable or a leaked secret.
    #[serde(default)]
    pub absent: Vec<String>,
}

/// What can go wrong while loading a case.
#[derive(Debug, thiserror::Error)]
pub enum CaseError {
    /// The case file could not be read.
    #[error("reading the case {path}: {source}")]
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The case is not valid YAML, or does not match the expected shape.
    #[error("case {path} is invalid: {source}")]
    Parse {
        /// The offending path.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: serde_yaml_ng::Error,
    },
    /// The case parsed but cannot be run.
    #[error("case {path}: {reason}")]
    Unrunnable {
        /// The offending path.
        path: PathBuf,
        /// What is missing, and what to add.
        reason: String,
    },
}

impl Case {
    /// Loads a case from `path`, **and checks it can be run**.
    ///
    /// The two go together for the same reason `Scenario::load` validates: a loaded case
    /// is a usable case, and nothing downstream should have to remember a second call.
    pub fn load(path: &Path) -> Result<Self, CaseError> {
        let raw = std::fs::read_to_string(path).map_err(|source| CaseError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::load_str(&raw, path)
    }

    /// Parses a case from YAML, reporting `origin` in any error.
    ///
    /// Separate from [`Case::load`] so tests can use inline fixtures and still get
    /// messages that name something.
    pub fn load_str(yaml: &str, origin: &Path) -> Result<Self, CaseError> {
        let case: Self = serde_yaml_ng::from_str(yaml).map_err(|source| CaseError::Parse {
            path: origin.to_path_buf(),
            source,
        })?;
        case.check_runnable(origin)?;
        Ok(case)
    }

    /// Refuses a case that can never be invoked.
    ///
    /// A case with neither `run` nor `exec` parses cleanly and then does nothing, which
    /// is the worst outcome available: a green test that asserts about a program that
    /// never started.
    fn check_runnable(&self, origin: &Path) -> Result<(), CaseError> {
        if self.setup.run.is_none() && self.setup.exec.is_none() {
            return Err(CaseError::Unrunnable {
                path: origin.to_path_buf(),
                reason: "setup has neither `run` nor `exec`, so nothing would be \
                         invoked. Add `run: [...]` with the command line, or `exec:` \
                         with a project executable that prepares and runs it"
                    .to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
name: sync-refuses-a-dirty-repository
weight: 5
setup:
  run: ["node", "bin/sync.js", "--dry-run"]
fake:
  rules:
    - match: { bin: git, args_contain: "status --porcelain" }
      stdout: " M src/index.js"
    - match: {}
      exit: 127
      stderr: "unexpected call"
expect:
  exit_code: 1
  stderr:
    contains: ["dirty repository"]
  calls:
    git: 1
    gh: 0
"#;

    #[test]
    fn a_case_parses_from_yaml() {
        let case = Case::load_str(MINIMAL, Path::new("inline")).unwrap();

        assert_eq!(case.name, "sync-refuses-a-dirty-repository");
        assert_eq!(case.weight, 5);
        assert!(!case.allow_fail);
        assert_eq!(
            case.setup.run.as_deref(),
            Some(
                ["node", "bin/sync.js", "--dry-run"]
                    .map(String::from)
                    .as_slice()
            )
        );
        assert_eq!(case.expect.exit_code, Some(1));
        assert_eq!(
            case.expect.stderr.as_ref().unwrap().contains,
            vec!["dirty repository"]
        );
        assert_eq!(case.expect.calls.as_ref().unwrap()["gh"], 0);
    }

    #[test]
    fn the_fake_scenario_is_the_engine_s_own_type() {
        let case = Case::load_str(MINIMAL, Path::new("inline")).unwrap();
        let scenario = case.fake.as_ref().unwrap();

        assert_eq!(scenario.rules.len(), 2);
        assert!(
            scenario.validate().is_ok(),
            "the core reuses gaveldrop-fake's Scenario rather than redeclaring it, so \
             the catch-all rule is validated by the engine that will consume it"
        );
    }

    #[test]
    fn allow_fail_defaults_to_false() {
        let yaml = "name: t\nweight: 1\nsetup: { run: [\"true\"] }\nexpect: { exit_code: 0 }\n";
        let case = Case::load_str(yaml, Path::new("inline")).unwrap();

        assert!(
            !case.allow_fail,
            "tolerating a failure must be opted into, never inherited by omission"
        );
    }

    #[test]
    fn the_core_keeps_unknown_setup_keys_opaque() {
        let yaml = r#"
name: t
weight: 1
setup:
  exec: ./prepare.sh
  pattern: ring
  agents: [t-charlie, t-alpha]
expect: { exit_code: 0 }
"#;
        let case = Case::load_str(yaml, Path::new("inline")).unwrap();

        assert_eq!(case.setup.exec.as_deref(), Some("./prepare.sh"));
        assert!(
            case.setup.extra.contains_key("pattern") && case.setup.extra.contains_key("agents"),
            "the core understands only `run` and `exec`; everything else must survive \
             untouched so it can reach the setup hook"
        );
        assert!(case.setup.run.is_none());
    }

    #[test]
    fn a_typo_outside_setup_is_refused() {
        let yaml = "name: t\nweight: 1\nsetup: { run: [\"true\"] }\nexpectt: { exit_code: 0 }\n";
        let error = Case::load_str(yaml, Path::new("inline")).unwrap_err();

        assert!(
            error.to_string().contains("expectt") || error.to_string().contains("expect"),
            "a misspelled key must fail at load time and name itself, not be ignored \
             into a case that silently asserts nothing: {error}"
        );
    }

    #[test]
    fn a_missing_required_key_is_refused() {
        let yaml = "weight: 1\nsetup: { run: [\"true\"] }\nexpect: { exit_code: 0 }\n";
        let error = Case::load_str(yaml, Path::new("inline")).unwrap_err();
        assert!(error.to_string().contains("name"), "message was: {error}");
    }

    #[test]
    fn a_case_loads_from_a_file_and_the_message_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dirty.yaml");
        std::fs::write(&path, MINIMAL).unwrap();
        assert_eq!(
            Case::load(&path).unwrap().name,
            "sync-refuses-a-dirty-repository"
        );

        let missing = dir.path().join("never-written.yaml");
        let error = Case::load(&missing).unwrap_err();
        assert!(
            error.to_string().contains("never-written.yaml"),
            "an error message must name the offender: {error}"
        );
    }

    #[test]
    fn a_case_without_a_way_to_run_is_refused() {
        let yaml = "name: t\nweight: 1\nsetup: {}\nexpect: { exit_code: 0 }\n";
        let error = Case::load_str(yaml, Path::new("inline")).unwrap_err();

        assert!(
            error.to_string().contains("run") && error.to_string().contains("exec"),
            "a case with neither `run` nor `exec` can never be invoked; say so at load \
             time and name both keys: {error}"
        );
    }
}
