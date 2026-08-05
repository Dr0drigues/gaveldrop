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
///
/// **Building one in Rust? End the literal with `..Default::default()`.** `Default` is derived for
/// exactly that: a conformance kit's factory constructs a `Case` by hand, and without the idiom every
/// field added here breaks it. Deriving it does not weaken the format — `name` and `weight` are still
/// required of a document, because neither is `#[serde(default)]`. See the same note on [`Setup`].
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
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
    /// How many seconds this **case** may run before its subject is killed. `0` is refused.
    ///
    /// Overrides the project's `timeout:`, and exists for the one case that legitimately takes longer
    /// than the rest — raising the project default for all of them would give every other case a
    /// guard that no longer guards anything.
    ///
    /// **A case with `steps:` spends this once, across all of them.** Each exchange gets what the ones
    /// before it left, and the exchanges after the one that spends the budget are not attempted. Per
    /// exchange the number would mean something else entirely: four blocking exchanges would announce
    /// two seconds and hold for eight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
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
    ///
    /// "As a whole" is not one rule for every key. `stdout` and `stderr` are every exchange's
    /// concatenated in order, `calls` is every exchange's added up, and `exit_code` is the **last**
    /// exchange's alone. A step's own `expect:` is checked against what that exchange produced, which is
    /// the only place a per-exchange failure can be caught.
    ///
    /// The exchanges share one isolation — the same root, `HOME`, faked tools and journal — which is
    /// what makes write-then-read work. They also share the case's `timeout:` as one budget rather than
    /// getting it each.
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
/// The core understands exactly three keys: `run`, `exec` and `env`. **Everything else is
/// opaque** and travels untouched into the setup hook, which is what lets a project
/// write its own vocabulary here without the core learning any domain words.
///
/// **Constructing one? End the literal with `..Default::default()`.** This gains fields — six
/// arrived across four releases — and an exhaustive literal stops compiling on the next one. Not
/// `#[non_exhaustive]`, deliberately: that would forbid the literal entirely, even with functional
/// update syntax, and force every consumer through mutation for a guarantee four words already buy.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct Setup {
    /// The command line to invoke, argument by argument. No shell is involved, so there
    /// are no quoting rules to remember.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<Vec<String>>,
    /// A project executable that prepares the isolated directory. It receives this whole
    /// block as JSON on its standard input.
    ///
    /// **Two directories, and they are not the same one.** The path is resolved against your
    /// repository, because that is where the script lives; the script then *runs* with the isolated
    /// root as its working directory, because that is what it is there to prepare. So
    /// `exec: ./tests/prepare.sh` finds your file, and a relative path written **inside** that
    /// script lands in the isolation. See `docs/hooks.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec: Option<String>,
    /// Environment variables the subject must see, on top of the ones isolation defines.
    ///
    /// For a subject that reads its configuration from the environment — a module guarded by
    /// `MYTOOL_FEATURE=true`, a tool locating itself through `$MYTOOL_DIR`. Without this such a
    /// subject cannot be invoked at all, which is how this key came to exist.
    ///
    /// A value may name what isolation defines: `MYTOOL_DIR: "$GAVELDROP_PROJECT"`. A name it does
    /// not define is an **error**, not a literal — nothing here reaches a shell, so a stray `$TYPO`
    /// could only set the variable to something quietly wrong.
    ///
    /// Two refusals, both loud. A name isolation already defines — `HOME`, `PATH`, the `XDG_*` and
    /// `GAVELDROP_*` families — cannot be redefined here: a case that could point `HOME` back at the
    /// real one would take the load-bearing invariant with it. And a name the project's `clear_env:`
    /// asks to remove cannot be set either, because the adapters clear *after* they set, so it would
    /// vanish without a word.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Tools that must be **findable nowhere**, so a case can prove what happens without them.
    ///
    /// "Warns when the binary is missing" is half of what a guarded module or an optional
    /// integration does, and faking cannot express it: a fake is a symlink, so `command -v mytool`
    /// finds it and the tool is *present*. This is the other half.
    ///
    /// Needed because `PATH` inside the isolation ends with the inherited one — the isolation has to
    /// keep `sh` and the rest working. Without this key the verdict depends on the machine: the same
    /// case passes on a bare runner and fails on a laptop where the tool is installed.
    ///
    /// **It removes whole directories.** Hiding `posting` drops every `PATH` entry that contains an
    /// executable of that name, so anything else installed only there disappears with it. The case
    /// then fails loudly with a command not found, never silently.
    ///
    /// Naming a tool the project's `fake.bins` also lists is **allowed, and this case wins**: no
    /// symlink is laid down for it. `fake.bins` is a declaration about the suite, `hide` one about
    /// this case, and the more specific of the two decides. Refusing the pair — which this did at
    /// first — left a module with two branches untestable in one configuration, since the project
    /// fakes the tool for most cases and one case has to prove what happens without it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<String>,
    /// Every other key, kept verbatim for the setup hook.
    ///
    /// What the subject reads on its standard input.
    ///
    /// For a **filter** — `stdin` in, `stdout` out — which is the commonest shape a terminal tool
    /// takes and was not invocable at all: a case had to write
    /// `run: ["sh", "-c", "… < fixture"]`, putting logic in a file that is meant to hold facts.
    ///
    /// Written in the case rather than read from a file, deliberately. YAML's `|` carries as many
    /// lines as you like, and a case holding both its input and its expectation reads in one piece
    /// instead of sending you to another file for half the story:
    ///
    /// ```yaml
    /// setup:
    ///   stdin: |
    ///     {"level":"INFO","message":"ready"}
    ///     plain text line
    ///   run: ["./format-logs"]
    /// ```
    ///
    /// **Not interpolated.** `run` substitutes because it is a command line and `env` because it is
    /// configuration; input is *data*. A log line may legitimately contain `$HOME`, and expanding it
    /// would corrupt the very thing under test.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
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
    ///
    /// **On a case with `steps:` this is the last exchange's**, where its neighbours aggregate: `stdout`
    /// and `stderr` concatenate every exchange and `calls` adds them up. Two keys in one block with
    /// opposite rules, so it is worth saying — `exit_code: 0` holds on a run whose middle exchange
    /// exited 42, and a case that does not check its exchanges one by one can believe it has proved
    /// nothing failed. An exchange failing on purpose partway through a scenario is a legitimate case,
    /// which is why the definition stands.
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
    ///
    /// Resolved against your repository and run with the isolated root as its working directory,
    /// exactly like `setup.exec` — two directories, which is worth knowing before a relative path
    /// inside the script surprises you.
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
    /// Groups of values that must each be a word of **one single line**.
    ///
    /// `line_includes: [["KUBE", "active"]]` holds when some line has both `KUBE` and `active` among
    /// its whitespace-separated words.
    ///
    /// **`contains` says a fragment exists somewhere, never that two of them belong together.** A
    /// consumer inverted every status in a `MODULE / STATUS` table — one `if enabled` flipped — and
    /// their case asserting `contains: ["KUBE", "DOCKER", "active", "inactive"]` kept passing: all four
    /// words were still present and the output was entirely wrong. Found by injecting that defect
    /// deliberately, to measure what the suite caught.
    ///
    /// **Words rather than substrings, and that is the half that makes it bite.** `inactive` *contains*
    /// `active`, so a substring match on the inverted row would have held too — the same trap
    /// `args_include` exists for one crate over. `include` means a whole value in this format;
    /// `contain` means a substring.
    ///
    /// The only answer available before was freezing the line with its padding —
    /// `contains: ["KUBE         active"]` — which works and makes changing `{:<12}` to `{:<14}`, a
    /// presentation decision, fail a test about behaviour. `docs/writing-cases.md` says that is what
    /// gets a case deleted rather than maintained.
    ///
    /// Order within the line is not checked, and neither is spacing. A failure names the line that came
    /// closest and what it lacked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub line_includes: Vec<Vec<String>>,
    /// The whole thing, exactly — for a value rather than a message.
    ///
    /// `contains` is close enough for prose and **states the opposite of what it checks** for a
    /// measurement: a case counting lines and asserting `contains: ["2"]` passes on a result of
    /// `12`. That is not a weak assertion, it is a false one, and it slips past the rule that every
    /// case must be able to fail.
    ///
    /// **One trailing newline is ignored on both sides.** A shell subject emits one almost always
    /// and a case never writes one, so comparing to the byte would fail every first attempt for a
    /// reason nothing on the page explains. Any other difference in whitespace counts, and when
    /// whitespace is the *only* difference the failure says so rather than showing two values that
    /// look identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<String>,
    /// Strips terminal escape sequences before comparing, for a subject that colours its output.
    ///
    /// A formatter that wraps *every field* in codes —
    /// `\x1b[2m08:00:00.123\x1b[0m \x1b[1;32mINFO\x1b[0m …` — breaks a `contains:` on the escapes
    /// sitting between the words. Writing them into the expectation works and is unreadable, which
    /// costs the first property to buy the assertion.
    ///
    /// **Off by default, and that is the decision rather than an omission.** A case may legitimately
    /// want to prove a colour *is* there, or is not — "no colour when the output is not a terminal"
    /// is the first thing worth asserting about a terminal tool. Stripping always would destroy that
    /// silently.
    ///
    /// Applies to `contains`, `absent` and `equals` alike, and only to the comparison: the
    /// observation keeps what the subject really wrote, exactly as a header keeps the spelling it
    /// arrived with. A failure shows the stripped text, since showing escapes you asked to ignore
    /// would explain nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ignore_ansi: bool,
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
    /// A key under `fake:` that the fake engine does not read.
    ///
    /// Loud because the quiet version was dangerous. `flatten` forbids `deny_unknown_fields` on
    /// a rule, and `Match` omits it so a project can compose its own criterion — so an unknown
    /// key used to be dropped, and dropping the only key of a `match:` leaves the empty match,
    /// which is the catch-all. The rule then answered every call, the rules after it were
    /// unreachable, and the catch-all check approved.
    #[error(
        "case {path}: `{at}` holds `{key}`, which the fake engine does not read{}. Known here: \
         {}. If `{key}` is your project's own vocabulary, your fake owns the whole scenario — put \
         it under `setup:`, which the core keeps opaque, and read it from your adapter",
        if at.ends_with("match") {
            ". An unknown criterion is not ignored, it leaves the match empty — and an empty \
             match is the catch-all, so this rule would answer every call and the rules after it \
             would never be reached"
        } else {
            ""
        },
        known.join(", ")
    )]
    UnknownFakeKey {
        /// The offending case.
        path: PathBuf,
        /// Where in the document, such as `fake.rules[0].match`.
        at: String,
        /// The key nothing reads.
        key: String,
        /// What was allowed at that position.
        known: &'static [&'static str],
    },
    /// The case parsed, and its fake could never prove anything.
    ///
    /// Checked here rather than left to the fake at call time, which is what `Scenario::validate`
    /// says in its own doc: you want to know before the first call rather than at the twentieth.
    /// Without this the case failed with `exit 125` on its first invocation, and a reader had a
    /// number to interpret instead of a sentence to act on.
    #[error("case {path}: {source}")]
    Scenario {
        /// The offending path.
        path: PathBuf,
        /// Why the scenario cannot work.
        #[source]
        source: gaveldrop_fake::NoCatchAll,
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

        if case.fake.is_some() {
            refuse_unknown_fake_keys(yaml).map_err(|(at, key, known)| {
                CaseError::UnknownFakeKey {
                    path: origin.to_path_buf(),
                    at,
                    key,
                    known,
                }
            })?;
        }

        if let Some(scenario) = &case.fake {
            scenario.validate().map_err(|source| CaseError::Scenario {
                path: origin.to_path_buf(),
                source,
            })?;
        }

        Ok(case)
    }
}

/// Where an unknown key was found, what it was, and what was allowed there.
type UnknownKey = (String, String, &'static [&'static str]);

/// Refuses a key under `fake:` that the fake engine does not read.
///
/// `deny_unknown_fields` cannot do this job: the response is `flatten`ed into the rule, and serde
/// forbids the two together. `Match` goes further and omits it deliberately, so a project can
/// compose its own criterion on top. Both were the right calls, and the consequence is that the
/// refusing has to happen here, against the key lists those types publish.
///
/// Left undone this is not a dropped field, it is a changed meaning. A `match:` whose only key is
/// unknown becomes the empty match, which **is** the catch-all — so the rule stops selecting and
/// starts answering every call, the rules after it become unreachable, and `validate` sees a
/// catch-all and approves. The case loads, runs, and proves nothing.
///
/// Works on the document rather than the parsed value because by the time serde is done the key
/// is gone.
fn refuse_unknown_fake_keys(yaml: &str) -> Result<(), UnknownKey> {
    let document: serde_yaml_ng::Value = match serde_yaml_ng::from_str(yaml) {
        Ok(document) => document,
        // The case already parsed into `Case`, so this cannot fail; and if it somehow did,
        // refusing nothing is right — the parse error is the better diagnostic.
        Err(_) => return Ok(()),
    };

    let Some(fake) = document.get("fake") else {
        return Ok(());
    };

    check_keys(fake, "fake", gaveldrop_fake::Scenario::KEYS)?;

    let Some(serde_yaml_ng::Value::Sequence(rules)) = fake.get("rules") else {
        return Ok(());
    };

    for (index, rule) in rules.iter().enumerate() {
        let at = format!("fake.rules[{index}]");
        check_keys(rule, &at, gaveldrop_fake::Rule::KEYS)?;

        if let Some(matcher) = rule.get("match") {
            check_keys(matcher, &format!("{at}.match"), gaveldrop_fake::Match::KEYS)?;
        }
    }

    Ok(())
}

/// The first key of `value` that is not in `known`, if any.
fn check_keys(
    value: &serde_yaml_ng::Value,
    at: &str,
    known: &'static [&'static str],
) -> Result<(), UnknownKey> {
    let serde_yaml_ng::Value::Mapping(mapping) = value else {
        return Ok(());
    };

    for key in mapping.keys() {
        let Some(name) = key.as_str() else { continue };
        if !known.contains(&name) {
            return Err((at.to_string(), name.to_string(), known));
        }
    }

    Ok(())
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
    fn a_fake_with_no_catch_all_is_refused_at_load_time() {
        let yaml = "name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nfake:\n  rules:\n    - match: { bin: git }\n      stdout: \"clean\"\nexpect:\n  exit_code: 0\n";

        let message = match Case::load_str(yaml, Path::new("inline")) {
            Ok(_) => panic!(
                "a scenario with no catch-all must not load. `Scenario::validate` says so in its \
                 own doc — at load time, not at call time — and nothing was calling it here, so \
                 the case failed with exit 125 at the first invocation instead"
            ),
            Err(error) => error.to_string(),
        };

        assert!(
            message.contains("catch-all"),
            "and the message must name what is missing rather than leaving a 125 to interpret: \
             {message}"
        );
        assert!(
            message.contains("inline"),
            "with the file, since a suite of fifty cases has fifty places to look: {message}"
        );
    }

    #[test]
    fn a_fake_with_a_catch_all_loads() {
        let yaml = "name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nfake:\n  rules:\n    - match: { bin: git }\n      stdout: \"clean\"\n    - match: {}\n      exit: 127\nexpect:\n  exit_code: 0\n";

        assert!(Case::load_str(yaml, Path::new("inline")).is_ok());
    }

    #[test]
    fn an_unknown_criterion_is_refused_rather_than_turned_into_a_catch_all() {
        let yaml = "name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nfake:\n  rules:\n    - match: { agent: t-writer }\n      stdout: \"done\"\n    - match: {}\n      exit: 127\nexpect:\n  exit_code: 0\n";

        let error = Case::load_str(yaml, Path::new("inline")).unwrap_err();
        let text = error.to_string();

        assert!(
            text.contains("fake.rules[0].match") && text.contains("agent"),
            "the position and the key both have to be there, or the reader is left grepping \
             their own case: {text}"
        );
        assert!(
            text.contains("catch-all"),
            "the consequence is the point. An unknown criterion left the match empty, and an \
             empty match is the catch-all — so this rule answered every call and the one after \
             it was dead. That is a green case proving nothing, not a dropped field: {text}"
        );
        assert!(
            text.contains("bin") && text.contains("stdin_contains"),
            "and what was allowed instead, since a typo is the common case: {text}"
        );
        assert!(
            text.contains("setup:"),
            "a project whose criterion is genuinely its own needs to be told where it goes, \
             or the only advice this message gives is `give up`: {text}"
        );
    }

    #[test]
    fn a_typo_in_a_response_is_refused_too() {
        let yaml = "name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nfake:\n  rules:\n    - match: {}\n      stdoutt: \"clean\"\nexpect:\n  exit_code: 0\n";

        let error = Case::load_str(yaml, Path::new("inline")).unwrap_err();
        let text = error.to_string();

        assert!(
            text.contains("fake.rules[0]") && text.contains("stdoutt"),
            "the response is flattened into the rule, so `deny_unknown_fields` cannot sit on \
             either — and a silently ignored `stdoutt` is a fake answering nothing: {text}"
        );
        assert!(
            !text.contains("catch-all"),
            "this one is not a criterion, so the catch-all sentence would be a red herring: \
             {text}"
        );
    }

    /// A key of the project's `fake:` block is still refused on a case's.
    ///
    /// This test used to make its point with `bins`, on the reasoning that naming the shadowed tools
    /// is a statement about the suite. A consumer showed the cost of that: a project's own binary
    /// cannot be faked suite-wide without breaking every case whose subject runs it, so the one case
    /// proving a delegation had nothing to intercept. `bins` is a case's key now, and
    /// `no_passthrough` — which decides what a whole environment may reach — is not.
    #[test]
    fn a_project_only_key_on_the_fake_block_is_refused() {
        let yaml = "name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nfake:\n  no_passthrough: true\n  rules:\n    - match: {}\n      exit: 0\nexpect:\n  exit_code: 0\n";

        let error = Case::load_str(yaml, Path::new("inline")).unwrap_err();

        assert!(
            error.to_string().contains("no_passthrough"),
            "whether a rule may reach the real tool is a property of where the suite runs — CI with \
             no credentials — and is usually set from the environment. Ignored on a case, it would \
             read as though the case had disarmed it: {error}"
        );
    }

    /// `fake.bins` on a case is accepted, and means the tools this case shadows on top of the suite's.
    #[test]
    fn a_case_may_name_the_binaries_it_shadows_itself() {
        let yaml = "name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nfake:\n  bins: [mytool]\n  rules:\n    - match: {}\n      exit: 0\nexpect:\n  exit_code: 0\n";

        let case = Case::load_str(yaml, Path::new("inline")).unwrap();

        assert_eq!(
            case.fake.expect("the block is there").bins,
            vec!["mytool".to_string()],
            "a project's own binary is the one thing `fake.bins` in gaveldrop.yaml cannot express: \
             suite-wide it breaks every case whose subject runs it for real"
        );
    }

    #[test]
    fn every_key_the_fake_engine_does_read_is_accepted() {
        let yaml = "name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nfake:\n  render: ./render.sh\n  rules:\n    - match: { bin: git, args_contain: status, stdin_contains: x, call: 1 }\n      stdout: out\n      stderr: err\n      exit: 1\n      exec: real\n      latency_ms: 5\n      status: 200\n      headers: { content-type: text/plain }\n    - match: {}\n      exit: 127\nexpect:\n  exit_code: 0\n";

        assert!(
            Case::load_str(yaml, Path::new("inline")).is_ok(),
            "the refusal is only worth having if it lets through everything the engine reads. \
             A list that drifted behind the types would refuse a legitimate case"
        );
    }

    #[test]
    fn a_case_with_no_fake_block_still_loads() {
        let yaml = "name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nexpect:\n  exit_code: 0\n";

        assert!(
            Case::load_str(yaml, Path::new("inline")).is_ok(),
            "a case that fakes nothing has no scenario to validate, and most cases are that"
        );
    }

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
