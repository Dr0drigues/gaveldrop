//! The scenario: rules, each pairing a criterion with a response.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Invocation;

/// Criterion selecting a rule. Every field is optional; a `Match` whose fields are
/// all absent is the **catch-all**.
///
/// Deliberately without `deny_unknown_fields`: a project composes its own criterion
/// on top of this one —
/// `struct MyMatch { #[serde(flatten)] core: Match, agent: Option<String> }` — and
/// `flatten` is incompatible with rejecting unknown fields. Catching typos is the
/// core's job, at case load time, against the JSON schema.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Match {
    /// Name of the faked binary, as the caller spelled it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<String>,
    /// Substring searched for in the arguments joined by spaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args_contain: Option<String>,
    /// Substring searched for in standard input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin_contains: Option<String>,
    /// Rank of the call, 1-indexed, for the counter key in force.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<u32>,
}

impl Match {
    /// True when this criterion is not one: it matches everything. That is the
    /// catch-all.
    pub fn is_catch_all(&self) -> bool {
        self.bin.is_none()
            && self.args_contain.is_none()
            && self.stdin_contains.is_none()
            && self.call.is_none()
    }

    /// True when this criterion needs standard input to have been read.
    pub fn needs_stdin(&self) -> bool {
        self.stdin_contains.is_some()
    }

    /// True when the call satisfies **every** criterion present. Criteria are
    /// cumulative; none of them is an `or`.
    pub fn matches(&self, inv: &Invocation, call: u32) -> bool {
        if let Some(want) = &self.bin
            && want != &inv.bin
        {
            return false;
        }
        if let Some(needle) = &self.args_contain
            && !inv.args_joined().contains(needle.as_str())
        {
            return false;
        }
        if let Some(needle) = &self.stdin_contains
            && !inv.stdin.contains(needle.as_str())
        {
            return false;
        }
        if let Some(want) = self.call
            && want != call
        {
            return false;
        }
        true
    }
}

/// What the fake does when a rule applies.
///
/// The modes are exclusive. `exec` wins over `stdout`/`stderr`: delegating means
/// letting the other program do the writing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Response {
    /// What the fake writes on its standard output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    /// What the fake writes on its standard error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    /// Exit code of the fake. Absent means `0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    /// `real` passes through to the real binary, found further along `PATH`. Any
    /// other value is the path of a project executable to delegate to, with the same
    /// arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec: Option<String>,
    /// Wait before responding, in milliseconds. Meant to reproduce a
    /// timing-sensitive sequence, not to slow things down for its own sake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// HTTP status, for a dependency faked behind the HTTP door. Absent means `200`.
    ///
    /// Separate from `exit` rather than reusing it: an exit code is a number between 0 and
    /// 255 that means "did the program succeed", and a status is a three-digit code that
    /// means something else entirely. Folding them together would make `exit: 404`
    /// meaningless at the binary door and `exit: 1` meaningless at this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Response headers, for the HTTP door. Ignored by the binary door.
    ///
    /// Present because a client that refuses a body without its `Content-Type` is not a
    /// client doing anything unusual, and a fake it rejects tests nothing.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl Response {
    /// True when this response passes through to the real binary.
    pub fn is_passthrough(&self) -> bool {
        self.exec.as_deref() == Some("real")
    }
}

/// One rule: a criterion and the response that goes with it.
///
/// Deliberately not generic over the criterion. A project needing its own criterion
/// declares its own `Rule` — it has its own response fields anyway — and reuses only
/// [`Match`] from this crate, flattened under its own. Making `Rule` generic would
/// mean carrying serde and schemars bounds on the parameter for a benefit nobody is
/// asking for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Rule {
    /// The criterion deciding whether this rule applies.
    #[serde(rename = "match", default)]
    pub matcher: Match,
    /// What to do when it does.
    #[serde(flatten)]
    pub response: Response,
}

/// The whole scenario, as it reaches the fake binary.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Scenario {
    /// Executable that dresses the selected response. It receives the rule and the
    /// call as JSON on its standard input, and returns the bytes to emit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<String>,
    /// The rules, in the order they are tried.
    pub rules: Vec<Rule>,
}

impl Scenario {
    /// Refuses a scenario with no catch-all.
    ///
    /// Meant to be called at load time, not at call time: a scenario with no
    /// catch-all is an error in the scenario, and you want to know before the first
    /// call rather than at the twentieth.
    pub fn validate(&self) -> Result<(), NoCatchAll> {
        require_catch_all(self.rules.iter().map(|rule| rule.matcher.is_catch_all()))
    }

    /// True when at least one rule needs standard input. Decides whether
    /// [`Invocation::from_env`] should read it.
    pub fn needs_stdin(&self) -> bool {
        self.rules.iter().any(|rule| rule.matcher.needs_stdin())
    }

    /// The first rule that applies, or `None` — which can only happen when
    /// [`Scenario::validate`] was not called.
    pub fn select(&self, inv: &Invocation, call: u32) -> Option<&Rule> {
        self.rules
            .iter()
            .find(|rule| rule.matcher.matches(inv, call))
    }
}

/// What can go wrong while loading a scenario.
#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    /// The core did not set the scenario path variable before invoking the fake.
    #[error("environment variable {0} is missing: the core must set it before invoking the fake")]
    MissingEnv(&'static str),
    /// The scenario file could not be read.
    #[error("reading the scenario {path}: {source}")]
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The scenario file is not valid YAML, or does not match the expected shape.
    #[error("scenario {path} is unreadable: {source}")]
    Parse {
        /// The offending path.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: serde_yaml_ng::Error,
    },
    /// The scenario parsed but has no catch-all.
    #[error("scenario {path}: {source}")]
    Invalid {
        /// The offending path.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: NoCatchAll,
    },
}

impl Scenario {
    /// Loads a scenario **and validates it**.
    ///
    /// The two go together on purpose: a loaded scenario is a usable scenario. Nothing
    /// in the calling code should have to remember to validate.
    pub fn load(path: &Path) -> Result<Self, ScenarioError> {
        let raw = std::fs::read_to_string(path).map_err(|source| ScenarioError::Io {
            path: path.to_path_buf(),
            source,
        })?;

        let scenario: Self =
            serde_yaml_ng::from_str(&raw).map_err(|source| ScenarioError::Parse {
                path: path.to_path_buf(),
                source,
            })?;

        scenario
            .validate()
            .map_err(|source| ScenarioError::Invalid {
                path: path.to_path_buf(),
                source,
            })?;

        Ok(scenario)
    }

    /// Loads the scenario designated by [`crate::env::SCENARIO`].
    pub fn from_env() -> Result<Self, ScenarioError> {
        let path = std::env::var_os(crate::env::SCENARIO)
            .ok_or(ScenarioError::MissingEnv(crate::env::SCENARIO))?;
        Self::load(Path::new(&path))
    }
}

/// A scenario with no catch-all.
#[derive(Debug, thiserror::Error)]
#[error(
    "scenario has no catch-all: add a `match: {{}}` rule last. Without it an \
     unexpected call would pass for an expected one, and the case would stop \
     proving anything"
)]
pub struct NoCatchAll;

/// Ok when at least one of the flags says "I am a catch-all".
///
/// Takes booleans rather than rules so that a project whose criterion is not
/// [`Match`] can reuse the check.
pub fn require_catch_all<I: IntoIterator<Item = bool>>(flags: I) -> Result<(), NoCatchAll> {
    if flags.into_iter().any(|is_catch_all| is_catch_all) {
        Ok(())
    } else {
        Err(NoCatchAll)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inv(bin: &str, args: &[&str], stdin: &str) -> Invocation {
        Invocation {
            bin: bin.to_string(),
            args: args.iter().map(|s| (*s).to_string()).collect(),
            stdin: stdin.to_string(),
        }
    }

    #[test]
    fn an_empty_match_is_a_catch_all_and_matches_everything() {
        let m = Match::default();
        assert!(m.is_catch_all());
        assert!(m.matches(&inv("git", &["status"], ""), 1));
        assert!(m.matches(&inv("kubectl", &[], ""), 42));
    }

    #[test]
    fn bin_discriminates_the_faked_binary() {
        let m = Match {
            bin: Some("git".into()),
            ..Default::default()
        };
        assert!(!m.is_catch_all());
        assert!(m.matches(&inv("git", &["status"], ""), 1));
        assert!(!m.matches(&inv("kubectl", &["get"], ""), 1));
    }

    #[test]
    fn args_contain_searches_the_joined_arguments() {
        let m = Match {
            args_contain: Some("get pods".into()),
            ..Default::default()
        };
        assert!(
            m.matches(&inv("kubectl", &["get", "pods", "-A"], ""), 1),
            "the needle spans two arguments, so matching must happen after joining"
        );
        assert!(!m.matches(&inv("kubectl", &["get", "svc"], ""), 1));
    }

    #[test]
    fn stdin_contains_searches_standard_input() {
        let m = Match {
            stdin_contains: Some("AGENT: alpha".into()),
            ..Default::default()
        };
        assert!(m.matches(&inv("claude", &[], "noise\nAGENT: alpha\n"), 1));
        assert!(!m.matches(&inv("claude", &[], "noise"), 1));
    }

    #[test]
    fn call_discriminates_the_call_rank() {
        let m = Match {
            call: Some(2),
            ..Default::default()
        };
        assert!(!m.matches(&inv("git", &[], ""), 1));
        assert!(m.matches(&inv("git", &[], ""), 2));
        assert!(!m.matches(&inv("git", &[], ""), 3));
    }

    #[test]
    fn criteria_are_cumulative() {
        let m = Match {
            bin: Some("git".into()),
            args_contain: Some("push".into()),
            call: Some(1),
            ..Default::default()
        };
        assert!(m.matches(&inv("git", &["push", "origin"], ""), 1));
        assert!(
            !m.matches(&inv("git", &["push", "origin"], ""), 2),
            "every present criterion must hold; none of them is an `or`"
        );
        assert!(!m.matches(&inv("git", &["pull"], ""), 1));
    }

    #[test]
    fn needs_stdin_is_true_only_when_a_criterion_uses_it() {
        assert!(!Match::default().needs_stdin());
        assert!(
            !Match {
                bin: Some("git".into()),
                ..Default::default()
            }
            .needs_stdin()
        );
        assert!(
            Match {
                stdin_contains: Some("x".into()),
                ..Default::default()
            }
            .needs_stdin()
        );
    }

    #[test]
    fn the_first_matching_rule_wins() {
        let scenario = Scenario {
            render: None,
            rules: vec![
                Rule {
                    matcher: Match {
                        bin: Some("git".into()),
                        ..Default::default()
                    },
                    response: Response {
                        stdout: Some("first".into()),
                        ..Default::default()
                    },
                },
                Rule {
                    matcher: Match::default(),
                    response: Response {
                        stdout: Some("catch-all".into()),
                        ..Default::default()
                    },
                },
            ],
        };

        let chosen = scenario.select(&inv("git", &[], ""), 1).unwrap();
        assert_eq!(chosen.response.stdout.as_deref(), Some("first"));

        let chosen = scenario.select(&inv("gh", &[], ""), 1).unwrap();
        assert_eq!(
            chosen.response.stdout.as_deref(),
            Some("catch-all"),
            "an unexpected call must reach the catch-all, not go unmatched"
        );
    }

    #[test]
    fn a_scenario_without_a_catch_all_is_refused() {
        assert!(require_catch_all([false, false]).is_err());
        assert!(require_catch_all([false, true]).is_ok());
    }

    #[test]
    fn a_scenario_parses_from_yaml() {
        let yaml = r#"
render: ./tests/fakes/claude.sh
rules:
  - match: { bin: kubectl, args_contain: "get pods" }
    stdout: "pod-a  Running"
  - match: { bin: sops }
    exec: real
  - match: {}
    exit: 127
    stderr: "unexpected call"
"#;
        let scenario: Scenario = serde_yaml_ng::from_str(yaml).unwrap();

        assert_eq!(scenario.render.as_deref(), Some("./tests/fakes/claude.sh"));
        assert_eq!(scenario.rules.len(), 3);
        assert_eq!(scenario.rules[0].matcher.bin.as_deref(), Some("kubectl"));
        assert!(scenario.rules[1].response.is_passthrough());
        assert_eq!(scenario.rules[2].response.exit, Some(127));
        assert!(scenario.rules[2].matcher.is_catch_all());
        assert!(scenario.validate().is_ok());
        assert!(
            !scenario.needs_stdin(),
            "no rule uses stdin_contains, so the fake must not read standard input"
        );
    }

    #[test]
    fn a_yaml_scenario_without_a_catch_all_fails_validation() {
        let yaml = "rules:\n  - match: { bin: git }\n    stdout: ok\n";
        let scenario: Scenario = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(scenario.validate().is_err());
    }

    #[test]
    fn a_scenario_loads_from_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scenario.yaml");
        std::fs::write(
            &path,
            "rules:\n  - match: { bin: git }\n    stdout: ok\n  - match: {}\n    exit: 127\n",
        )
        .unwrap();

        let scenario = Scenario::load(&path).unwrap();
        assert_eq!(scenario.rules.len(), 2);
    }

    #[test]
    fn a_loaded_scenario_is_validated_in_the_same_breath() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scenario.yaml");
        std::fs::write(&path, "rules:\n  - match: { bin: git }\n    stdout: ok\n").unwrap();

        let error = Scenario::load(&path).unwrap_err();
        assert!(
            error.to_string().contains("catch-all"),
            "loading must refuse a scenario with no catch-all rather than wait for the \
             first call, and the message must say what to add: {error}"
        );
    }

    #[test]
    fn a_missing_file_names_the_path_in_its_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("never-written.yaml");
        let error = Scenario::load(&path).unwrap_err();
        assert!(
            error.to_string().contains("never-written.yaml"),
            "an error message must name the offender: {error}"
        );
    }

    #[test]
    fn invalid_yaml_names_the_path_in_its_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.yaml");
        std::fs::write(&path, "rules: [ this is not: a rule\n").unwrap();
        let error = Scenario::load(&path).unwrap_err();
        assert!(
            error.to_string().contains("broken.yaml"),
            "an error message must name the offender: {error}"
        );
    }
}
