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
    ///
    /// **Required, even when empty.** A case whose assertions are all per step writes `expect: {}`,
    /// which is two characters of noise buying a real guarantee: a case cannot arrive with no
    /// expectations at all and pass. `allow_fail` is required for the same reason — a claim this
    /// project needs to be deliberate is never made by omission.
    pub expect: Expect,
    /// Exchanges with the subject, in order, each with its own expectations.
    ///
    /// Invoking a subject more than once is observable of any process — you can run a binary
    /// twice — so this is part of the format rather than one technology's vocabulary. `expect`
    /// above still describes what the run produced **as a whole**; a step describes one exchange.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
}

/// One exchange with the subject.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// What this exchange is for, in the reader's words.
    ///
    /// Optional, and worth writing: it is what a failure names instead of an index alone, so a
    /// reader is not left counting lines in the document to find which one broke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// How to perform this exchange. Opaque to the core, exactly like `setup` past `run`.
    ///
    /// *How many* exchanges there are is the format's business; *how* one is performed belongs to
    /// whichever adapter claims the case.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub request: BTreeMap<String, serde_json::Value>,
    /// What this exchange must produce.
    #[serde(default)]
    pub expect: Expect,
    /// Values to name from this exchange's response, for later ones to substitute.
    ///
    /// A name to a JSON path: `capture: { order_id: data.order.id }` makes `$order_id` usable in
    /// every **later** step. Naming and substituting is all a case may do with a value — see the
    /// invariant in `ARCHITECTURE.md`. Anything that would compute belongs in a hook.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub capture: BTreeMap<String, String>,
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
    /// A project executable that checks what the core cannot.
    ///
    /// It receives the observations as JSON on its standard input and answers
    /// `{"ok": bool, "diffs": [...]}`. See `docs/hooks.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec: Option<String>,
    /// Named invariants that must hold. The names come from the project configuration, so a
    /// name nothing declares is a case failure rather than a silent skip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invariants: Vec<String>,
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
    /// The status the response must carry.
    ///
    /// In the core rather than in an adapter, and the placement rule is what puts it there. An
    /// extension holds what **one** technology can produce; HTTP is answered identically by a
    /// service in Node, Rust, Python, Java or Kotlin. That is what lets a case be rewritten in
    /// another language without touching a single expectation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Assertions on response headers, by name. Names are matched case-insensitively.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, TextExpectation>>,
    /// Assertions on the response body.
    ///
    /// The same shape as `stdout`, deliberately: a reader who knows one knows both, and `absent`
    /// keeps its truncation and its indexed assertion paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<TextExpectation>,
    /// Whether this exchange must have written nothing at all.
    ///
    /// This is what `idempotent:` turned out to be. Two identical steps where the second declares
    /// `no_new_files: true` says exactly what a boolean would have hidden: the subject runs twice,
    /// and what is compared is the file tree. A keyword would have concealed both.
    ///
    /// Also useful on its own — a tool that claims to read without writing can now be held to it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_new_files: bool,
    /// Assertions on values inside a JSON body, by dotted path — `data.order.id`, `items.0.sku`.
    ///
    /// Exists because GraphQL answers `200` for a failed operation and puts the failure in
    /// `errors`, so a status is not enough; and because a substring match on a serialisation is
    /// sensitive to spacing and key order. Keys and numeric indices only: no wildcards, no filters,
    /// no recursion. A query language here would be computation, and computation belongs in a hook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<BTreeMap<String, TextExpectation>>,
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
}

impl Case {
    /// Loads a case from `path`.
    ///
    /// **Loading does not decide whether the case can be invoked.** It once did, refusing any
    /// `setup` without `run` or `exec` — the concern was right and still is: a case that parses
    /// and then invokes nothing is a green test asserting about a program that never started. But
    /// with more than one adapter, `run` and `exec` are no longer the criterion. Only the adapters
    /// know what they can run, and `case` must not depend on them.
    ///
    /// So the refusal moved to [`crate::adapters::select`], which knows the whole registry and can
    /// name the keys it did find. One place decides, and it is the place with the knowledge.
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
        Ok(case)
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
    fn a_case_with_no_way_to_run_loads_and_is_refused_later() {
        let yaml = "name: t\nweight: 1\nsetup: {}\nexpect: { exit_code: 0 }\n";

        Case::load_str(yaml, Path::new("inline")).expect(
            "whether a case can be invoked is no longer knowable here: only the adapters know, \
             and `case` must not depend on them. The refusal lives in `adapters::select`, which \
             is where `a_case_claimed_by_no_one_names_the_case_and_says_what_would_work` \
             asserts it",
        );
    }
}
