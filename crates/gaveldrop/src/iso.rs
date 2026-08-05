//! The isolated environment a case runs in.
//!
//! This module carries the load-bearing invariant of the whole project: **a case never
//! sees the real home directory.** A defect here means the suite silently corrupts the
//! actual configuration of whoever runs it. Every change is reviewed with that sentence
//! in mind.
//!
//! Isolation asks nothing of the subject under test. No variable to read, no test mode,
//! no injection point — only what a process is subjected to anyway: its environment and
//! its search path.

pub mod paths;
pub mod port;
pub mod snapshot;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use gaveldrop_fake::{Match, Response, Rule, Scenario};

use crate::Case;
use crate::iso::snapshot::{FileEffect, Snapshot};

/// A pristine directory, the environment that points into it, and the symlinks that put
/// the fake first on the search path.
pub struct Isolation {
    root: tempfile::TempDir,
    project_root: PathBuf,
    env: Vec<(String, OsString)>,
    cleared: Vec<String>,
    snapshot: Snapshot,
    limit: Option<std::time::Duration>,
}

/// What can go wrong while preparing isolation.
#[derive(Debug, thiserror::Error)]
pub enum IsoError {
    /// A directory or file inside the isolated root could not be created.
    #[error("preparing the isolated environment at {path}: {source}")]
    Io {
        /// The path that could not be created.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The scenario could not be serialised for the fake.
    #[error("writing the scenario for the fake: {0}")]
    Scenario(#[from] serde_yaml_ng::Error),
    /// Passthrough is refused here and a rule has nothing else to answer with.
    #[error(
        "this project refuses passthrough, and the rule for `{bin}` has no answer of its own. Give \
         it a `stdout` (and an `exit` if it should fail) so it can answer where the real tool \
         cannot run"
    )]
    NoFallback {
        /// The binary whose rule has no fallback.
        bin: String,
    },
    /// `setup.env` tries to redefine something isolation owns.
    #[error(
        "setup.env cannot set `{name}`: isolation defines it, and a case that could point it \
         elsewhere would undo the isolation it is running in. Isolation owns {available}"
    )]
    ReservedVariable {
        /// The variable the case tried to redefine.
        name: String,
        /// Everything isolation defines, comma-separated.
        available: String,
    },
    /// `setup.env` sets something the project's configuration asks to remove.
    #[error(
        "setup.env sets `{name}`, which this project's `clear_env:` asks to remove. An adapter \
         clears after it sets, so the value would disappear without a word. Take it out of \
         `clear_env` or out of the case — the two cannot both be meant"
    )]
    ClearedVariable {
        /// The contradicted variable.
        name: String,
    },
    /// A value in `setup.env` names something isolation does not define.
    #[error("setup.env `{name}`: {source}")]
    Variable {
        /// The variable being set.
        name: String,
        /// Why its value could not be resolved.
        #[source]
        source: crate::iso::paths::PathError,
    },
}

impl Isolation {
    /// Prepares everything a case needs to run, and nothing it could escape through.
    ///
    /// `faked_bins` names the tools to shadow; each gets a symlink to `fake_binary` in a
    /// directory placed first on `PATH`. `clear_env` names the variables to remove — see
    /// [`Isolation::cleared`] for why removing beats overriding.
    ///
    /// `project_root` is where the case's own files live, and it is **not** a hole in the
    /// isolation: nothing is copied in and nothing is writable there. It is carried because some
    /// subjects are files of the project rather than executables on `PATH` — a shell function has
    /// to be sourced from the repository to be the thing under test. See
    /// [`Isolation::project_root`].
    pub fn prepare(
        case: &Case,
        fake_binary: &Path,
        faked_bins: &[String],
        clear_env: &[String],
        project_root: &Path,
    ) -> Result<Self, IsoError> {
        Self::prepare_with(
            case,
            fake_binary,
            faked_bins,
            clear_env,
            project_root,
            false,
        )
    }

    /// Prepares isolation, optionally refusing to let any rule reach a real tool.
    ///
    /// `refuse_passthrough` is for an environment where the real tool cannot work — CI with no
    /// credentials and no network. A project then fakes there what it passes through on a laptop,
    /// without maintaining two sets of cases.
    pub fn prepare_with(
        case: &Case,
        fake_binary: &Path,
        faked_bins: &[String],
        clear_env: &[String],
        project_root: &Path,
        refuse_passthrough: bool,
    ) -> Result<Self, IsoError> {
        let root = tempfile::tempdir().map_err(|source| IsoError::Io {
            path: PathBuf::from("(temporary directory)"),
            source,
        })?;
        let base = root.path().to_path_buf();

        let bin_dir = base.join("bin");
        let state_dir = base.join("state");
        let config_home = base.join(".config");
        let data_home = base.join(".local/share");
        let state_home = base.join(".local/state");
        let cache_home = base.join(".cache");

        for dir in [
            &bin_dir,
            &state_dir,
            &config_home,
            &data_home,
            &state_home,
            &cache_home,
        ] {
            create_dir(dir)?;
        }

        for name in faked_bins {
            if case.setup.hide.contains(name) {
                continue;
            }
            let link = bin_dir.join(name);
            std::os::unix::fs::symlink(fake_binary, &link)
                .map_err(|source| IsoError::Io { path: link, source })?;
        }

        let scenario_path = base.join("scenario.yaml");
        write_scenario(&scenario_path, case.fake.as_ref(), refuse_passthrough)?;

        let port = crate::iso::port::reserve().map_err(|source| IsoError::Io {
            path: PathBuf::from("(a free port)"),
            source,
        })?;
        let fake_port = crate::iso::port::reserve().map_err(|source| IsoError::Io {
            path: PathBuf::from("(a free port for the faked service)"),
            source,
        })?;

        let journal = base.join("journal.jsonl");

        let inherited = without(
            &std::env::var_os("PATH").unwrap_or_default(),
            &case.setup.hide,
        );
        let searched = std::iter::once(bin_dir.clone()).chain(inherited);
        let path = std::env::join_paths(searched).map_err(|_| IsoError::Io {
            path: bin_dir.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a directory on PATH contains the path separator",
            ),
        })?;

        let env = vec![
            ("PATH".to_string(), path),
            ("HOME".to_string(), base.clone().into()),
            ("XDG_CONFIG_HOME".to_string(), config_home.into()),
            ("XDG_DATA_HOME".to_string(), data_home.into()),
            ("XDG_STATE_HOME".to_string(), state_home.into()),
            ("XDG_CACHE_HOME".to_string(), cache_home.into()),
            (
                gaveldrop_fake::env::SCENARIO.to_string(),
                scenario_path.into(),
            ),
            (gaveldrop_fake::env::STATE.to_string(), state_dir.into()),
            (gaveldrop_fake::env::JOURNAL.to_string(), journal.into()),
            (gaveldrop_fake::env::DIR.to_string(), base.into()),
            (
                gaveldrop_fake::env::CASE.to_string(),
                case.name.clone().into(),
            ),
            ("GAVELDROP_PORT".to_string(), port.to_string().into()),
            (
                "GAVELDROP_FAKE_PORT".to_string(),
                fake_port.to_string().into(),
            ),
            (
                "GAVELDROP_PROJECT".to_string(),
                absolute(project_root).into_os_string(),
            ),
        ];

        let env = with_the_cases_own(env, case, clear_env)?;

        Ok(Self {
            root,
            project_root: project_root.to_path_buf(),
            env,
            cleared: clear_env.to_vec(),
            snapshot: Snapshot::default(),
            limit: Some(std::time::Duration::from_secs(
                crate::config::DEFAULT_TIMEOUT_SECONDS,
            )),
        })
    }

    /// Records the tree as it stands, so [`Isolation::changes`] reports what the subject did
    /// rather than what setup left behind.
    ///
    /// Called after the setup hook and before invocation.
    pub fn snapshot(&mut self) {
        self.snapshot = Snapshot::take(self.root.path());
    }

    /// What the subject changed under the isolated root.
    pub fn changes(&self) -> Vec<FileEffect> {
        self.snapshot.changes_since(self.root.path())
    }

    /// The isolated root. Everything the case may touch lives under it.
    pub fn root(&self) -> &Path {
        self.root.path()
    }

    /// Where the fake appends its call journal.
    pub fn journal_path(&self) -> PathBuf {
        self.root.path().join("journal.jsonl")
    }

    /// The variables to set on the subject's process.
    pub fn env(&self) -> Vec<(String, OsString)> {
        self.env.clone()
    }

    /// The variables a case may use in a path, as strings.
    ///
    /// Exactly what isolation defines, and nothing from the environment of whoever runs the
    /// tests — that closed set is what keeps a case from depending on its runner.
    pub fn defined(&self) -> std::collections::BTreeMap<String, String> {
        self.env
            .iter()
            .map(|(key, value)| (key.clone(), value.to_string_lossy().into_owned()))
            .collect()
    }

    /// Where the case's own files live, for a subject that *is* a file of the project.
    ///
    /// A shell function must be sourced from the repository to be the thing under test, so a
    /// relative `source:` resolves against this and not against the isolated root. Reading a
    /// project file is not a breach of isolation; **writing** would be, and nothing here permits
    /// it — the subject still runs with the isolated root as its working directory, so anything it
    /// creates lands inside.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// The variables to **remove** from the subject's process.
    ///
    /// Removing, not overriding. A project that reads `MYTOOL_CONFIG_DIR` before looking
    /// at the home directory short-circuits isolation if that variable happens to sit in
    /// the environment of whoever runs the tests. Giving it a new value inside the root
    /// would work for that one project; removing it works for every project that has not
    /// been thought of yet.
    pub fn cleared(&self) -> &[String] {
        &self.cleared
    }

    /// The same isolation, with how long the subject may run before it is killed.
    ///
    /// A builder rather than a seventh constructor parameter: `prepare` has five and `prepare_with`
    /// six, and a project's own tests build one. Chained by the runner, which is where the project's
    /// setting and the case's override are resolved into one answer.
    #[must_use]
    pub fn with_limit(mut self, limit: Option<std::time::Duration>) -> Self {
        self.limit = limit;
        self
    }

    /// How long the subject may run before it is killed, or `None` for no limit.
    ///
    /// **An adapter has to pass this to whatever it spawns.** It is how a custom adapter gets the
    /// guard, and the built-in ones read it here too — the alternative was every adapter learning
    /// that a project setting and a case override both exist. [`crate::adapters::invoke`] takes it
    /// directly.
    ///
    /// Defaults to [`crate::config::DEFAULT_TIMEOUT_SECONDS`] rather than to no limit, so an
    /// isolation built by a test or by the conformance kit is guarded too. The unsafe value is never
    /// the one you get by not thinking about it.
    pub fn limit(&self) -> Option<std::time::Duration> {
        self.limit
    }

    /// The case's limit as one budget its exchanges share, counting from now.
    ///
    /// **An adapter performing `steps:` wants this rather than `limit()`.** Handing the limit to each
    /// exchange afresh multiplies it by their number: four blocking exchanges with `timeout: 2` held for
    /// eight seconds while the verdict printed "exits within 2.0s". `timeout:` says *case*, and the
    /// promise it buys — a suite does not hang — has to hold whatever the number of exchanges.
    ///
    /// Offered here because this is where an adapter already comes for the limit, and because a
    /// consumer's own adapter performing exchanges has exactly the same problem in exactly the same
    /// shape. See [`crate::adapters::Budget`] for the loop it goes in.
    pub fn budget(&self) -> crate::adapters::Budget {
        crate::adapters::Budget::of(self.limit)
    }
}

/// The inherited search path, minus every directory holding one of `hidden`.
///
/// Directory-wise rather than entry-wise because that is the only granularity `PATH` has. A shell
/// asking `command -v posting` walks the directories and reports the first hit; there is no way to
/// say "this directory, except one name". Shadowing does not work either — the fake is a symlink, so
/// it makes the tool *present*, which is the opposite of what a case needs here.
///
/// The consequence is real and belongs in the documentation rather than in a comment: hiding one
/// tool takes its neighbours with it. A case that then needs one of them fails with a command not
/// found, which is loud, and loud is the requirement.
///
/// Returns the directories rather than a joined string, so the caller composes the whole `PATH` in
/// one `join_paths`. Concatenating with a `:` by hand is how an **empty entry** appears when nothing
/// survives the filter — and an empty entry means the current directory to a shell, which here is
/// the isolated root the subject is writing into.
///
/// Kept a free function taking the path so the whole rule is testable without touching the
/// environment of a test process that runs in parallel with others.
fn without(inherited: &std::ffi::OsStr, hidden: &[String]) -> Vec<PathBuf> {
    std::env::split_paths(inherited)
        .filter(|directory| !hidden.iter().any(|name| holds(directory, name)))
        .collect()
}

/// True when `directory` holds something executable called `name`.
///
/// Only the executable bit counts: `command -v` skips a file it could not run and keeps looking, so
/// a non-executable file of the same name hides nothing.
fn holds(directory: &Path, name: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(directory.join(name))
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Adds the variables the case declared, refusing the two ways it could do harm quietly.
///
/// Folded into the isolation's own list rather than handed to each adapter, so every adapter gets
/// it without a line of change — they already apply `iso.env()` — and so no adapter can be the one
/// that forgets. It also means a case may name its own variable in an `expect.files` path, since
/// `defined()` is derived from this list.
///
/// The values are expanded against what isolation defines, in that order, so
/// `MYTOOL_DIR: "$GAVELDROP_PROJECT"` resolves. One case variable cannot name another: a case names
/// and substitutes, it never computes, and a chain of references is the beginning of computing.
fn with_the_cases_own(
    mut env: Vec<(String, OsString)>,
    case: &Case,
    clear_env: &[String],
) -> Result<Vec<(String, OsString)>, IsoError> {
    if case.setup.env.is_empty() {
        return Ok(env);
    }

    let defined: std::collections::BTreeMap<String, String> = env
        .iter()
        .map(|(key, value)| (key.clone(), value.to_string_lossy().into_owned()))
        .collect();

    for (key, pattern) in &case.setup.env {
        if defined.contains_key(key) {
            return Err(IsoError::ReservedVariable {
                name: key.clone(),
                available: defined.keys().cloned().collect::<Vec<_>>().join(", "),
            });
        }
        if clear_env.iter().any(|cleared| cleared == key) {
            return Err(IsoError::ClearedVariable { name: key.clone() });
        }

        let value =
            crate::iso::paths::expand(pattern, &defined).map_err(|source| IsoError::Variable {
                name: key.clone(),
                source,
            })?;
        env.push((key.clone(), value.into()));
    }

    Ok(env)
}

/// The project root as an absolute path.
///
/// The runner is usually given `.`, and a subject runs with the isolated root as its working
/// directory — so a relative project root composed into a command line points inside the isolation,
/// where nothing of the project exists. This is the same mistake the setup hooks made before they
/// canonicalised, arriving by a different route.
fn absolute(project_root: &Path) -> PathBuf {
    std::fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf())
}

/// Replaces every passthrough with the fallback its rule declared.
///
/// A rule with **no** fallback is refused rather than answered emptily. Substituting an empty response
/// would make the subject see silence where it expected a real tool's output — a wrong answer dressed
/// as a right one, and the kind of green case this project exists to prevent. A project turning
/// passthrough off has to say what each rule answers instead.
fn grounded(scenario: &mut Scenario) -> Result<(), IsoError> {
    for rule in &mut scenario.rules {
        if !rule.response.is_passthrough() {
            continue;
        }

        if rule.response.stdout.is_none() && rule.response.stderr.is_none() {
            return Err(IsoError::NoFallback {
                bin: rule
                    .matcher
                    .bin
                    .clone()
                    .unwrap_or_else(|| "(the catch-all)".to_string()),
            });
        }

        rule.response.exec = None;
    }

    Ok(())
}

/// Creates `dir` and its parents, naming it if that fails.
fn create_dir(dir: &Path) -> Result<(), IsoError> {
    std::fs::create_dir_all(dir).map_err(|source| IsoError::Io {
        path: dir.to_path_buf(),
        source,
    })
}

/// Writes the scenario the fake will read.
///
/// A case with no `fake:` block still gets a scenario: one made of nothing but a
/// catch-all that exits 127. That way an unexpected call is a loud, attributable failure
/// rather than the fake crashing over a missing file — and the case still proves the
/// subject called nobody.
fn write_scenario(
    path: &Path,
    declared: Option<&Scenario>,
    refuse_passthrough: bool,
) -> Result<(), IsoError> {
    let fallback = Scenario {
        render: None,
        rules: vec![Rule {
            matcher: Match::default(),
            response: Response {
                exit: Some(127),
                stderr: Some("gaveldrop: unexpected call, this case declares no fake".into()),
                ..Default::default()
            },
        }],
    };

    let mut scenario = declared.unwrap_or(&fallback).clone();
    if refuse_passthrough {
        grounded(&mut scenario)?;
    }

    let yaml = serde_yaml_ng::to_string(&scenario)?;
    std::fs::write(path, yaml).map_err(|source| IsoError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {

    /// A directory holding an executable `name`, plus a `neighbour` beside it.
    fn a_directory_with(name: &str) -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        for file in [name, "neighbour"] {
            let path = dir.path().join(file);
            std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    }

    fn joined(dirs: &[&Path]) -> OsString {
        std::env::join_paths(dirs).unwrap()
    }

    /// The directories of a joined path, which is what `without` returns.
    fn expected(path: &OsString) -> Vec<PathBuf> {
        std::env::split_paths(path).collect()
    }

    #[test]
    fn hiding_a_tool_removes_the_directory_that_holds_it() {
        let with_it = a_directory_with("posting");
        let without_it = tempfile::tempdir().unwrap();

        let filtered = without(
            &joined(&[with_it.path(), without_it.path()]),
            &["posting".to_string()],
        );

        assert_eq!(
            filtered,
            vec![without_it.path().to_path_buf()],
            "a shell asked `command -v posting` walks the directories and reports the first hit, \
             so the only way to make a name unfindable is to drop the directories holding it. \
             Shadowing would make it *present*, which is the opposite of what a case needs"
        );
    }

    #[test]
    fn hiding_a_tool_takes_its_neighbours_with_it() {
        let shared = a_directory_with("posting");

        let filtered = without(&joined(&[shared.path()]), &["posting".to_string()]);

        assert!(
            filtered.is_empty(),
            "`neighbour` lived in the same directory and is gone too. That is the documented cost \
             of the only granularity PATH has — and a case needing it fails with a command not \
             found, which is loud rather than silent"
        );
    }

    #[test]
    fn hiding_nothing_leaves_the_search_path_exactly_as_it_was() {
        let one = a_directory_with("posting");
        let two = tempfile::tempdir().unwrap();
        let original = joined(&[one.path(), two.path()]);

        assert_eq!(
            without(&original, &[]),
            expected(&original),
            "every case that does not use the key must be untouched, or this stops being additive"
        );
    }

    #[test]
    fn a_name_nowhere_on_the_path_changes_nothing() {
        let one = a_directory_with("posting");
        let original = joined(&[one.path()]);

        assert_eq!(
            without(&original, &["no-such-tool-anywhere".to_string()]),
            expected(&original),
            "hiding what was already absent is a legitimate case to write — the machine simply did \
             not have it — and it must not quietly empty the path"
        );
    }

    #[test]
    fn a_file_that_is_not_executable_hides_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("posting"), "not a program").unwrap();
        let original = joined(&[dir.path()]);

        assert_eq!(
            without(&original, &["posting".to_string()]),
            expected(&original),
            "`command -v` skips a file it could not run and keeps looking, so a non-executable of \
             the same name never made the tool findable. Dropping the directory for it would \
             remove working tools for no reason"
        );
    }

    #[test]
    fn a_case_can_hide_a_tool_the_project_fakes() {
        let outside = tempfile::tempdir().unwrap();
        let hidden = Case::load_str(
            "name: t\nweight: 1\nsetup:\n  hide: [git]\n  run: [\"true\"]\nexpect: { exit_code: 0 }\n",
            Path::new("inline"),
        )
        .unwrap();

        let iso = Isolation::prepare(
            &hidden,
            &fake_binary(outside.path()),
            &["git".to_string(), "gh".to_string()],
            &[],
            outside.path(),
        )
        .unwrap();

        assert!(
            !iso.root().join("bin/git").exists(),
            "`fake.bins` is a declaration about the suite and `hide` one about this case, so the \
             more specific wins. Refusing the pair — which this used to do — made a module with \
             two branches untestable in one configuration: the project fakes the tool for most \
             cases, and one case has to prove what happens without it"
        );
        assert!(
            iso.root().join("bin/gh").exists(),
            "and only the named tool is withheld: the rest of `fake.bins` is untouched"
        );
    }

    fn passthrough_case(with_fallback: bool) -> Case {
        let response = if with_fallback {
            "      exec: real\n      stdout: \"the fallback\"\n      exit: 0\n"
        } else {
            "      exec: real\n"
        };
        Case::load_str(
            &format!(
                "name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nfake:\n  rules:\n    - match: {{ bin: git }}\n{response}    - match: {{}}\n      exit: 127\nexpect: {{ exit_code: 0 }}\n"
            ),
            Path::new("inline"),
        )
        .unwrap()
    }

    fn written_scenario(case: &Case, refuse: bool) -> Result<String, IsoError> {
        let outside = tempfile::tempdir().unwrap();
        let iso = Isolation::prepare_with(
            case,
            &fake_binary(outside.path()),
            &[],
            &[],
            outside.path(),
            refuse,
        )?;
        Ok(std::fs::read_to_string(iso.root().join("scenario.yaml")).unwrap())
    }

    #[test]
    fn a_passthrough_rule_becomes_its_declared_fallback() {
        let scenario = written_scenario(&passthrough_case(true), true).unwrap();

        assert!(
            !scenario.contains("exec: real"),
            "CI has no credentials and no network for the real tool, so a project must be able to \
             fake there what it passes through on a laptop: {scenario}"
        );
        assert!(
            scenario.contains("the fallback"),
            "and the fallback the case declared is what it answers with: {scenario}"
        );
    }

    #[test]
    fn a_passthrough_rule_with_no_fallback_is_refused_rather_than_answered_emptily() {
        let refused = written_scenario(&passthrough_case(false), true);

        let message = match refused {
            Ok(scenario) => {
                panic!("a rule with nothing to answer must not appear to work: {scenario}")
            }
            Err(error) => error.to_string(),
        };
        assert!(
            message.contains("git"),
            "naming the rule is the whole diagnostic: a project turning passthrough off has to know \
             which rule now needs a `stdout`: {message}"
        );
        assert!(
            message.contains("stdout"),
            "and what to add. Substituting an empty response would make the subject see silence \
             where it expected a real tool's output, which is a wrong answer dressed as a right \
             one: {message}"
        );
    }

    #[test]
    fn passthrough_survives_when_the_project_does_not_refuse_it() {
        let scenario = written_scenario(&passthrough_case(false), false).unwrap();

        assert!(
            scenario.contains("exec: real"),
            "the switch is opt-in: a laptop run keeps reaching the real tool: {scenario}"
        );
    }

    #[test]
    fn the_case_document_is_never_rewritten() {
        let case = passthrough_case(true);
        let before = format!("{:?}", case.fake);
        let _ = written_scenario(&case, true).unwrap();

        assert_eq!(
            format!("{:?}", case.fake),
            before,
            "the override applies to the scenario handed to the fake, never to the document. A case \
             that meant different things depending on where it ran would be unreadable"
        );
    }
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn case() -> Case {
        Case::load_str(
            "name: t\nweight: 1\nsetup: { run: [\"true\"] }\nexpect: { exit_code: 0 }\n",
            Path::new("inline"),
        )
        .unwrap()
    }

    fn fake_binary(dir: &Path) -> PathBuf {
        let path = dir.join("gaveldrop-fake");
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn env_of(iso: &Isolation) -> std::collections::BTreeMap<String, String> {
        iso.env()
            .into_iter()
            .map(|(key, value)| (key, value.to_string_lossy().into_owned()))
            .collect()
    }

    #[test]
    fn the_home_directory_points_inside_the_isolated_root() {
        let outside = tempfile::tempdir().unwrap();
        let iso = Isolation::prepare(
            &case(),
            &fake_binary(outside.path()),
            &[],
            &[],
            outside.path(),
        )
        .unwrap();
        let env = env_of(&iso);

        let root = iso.root().to_string_lossy().into_owned();
        for key in ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME"] {
            assert!(
                env[key].starts_with(&root),
                "{key} must point inside the isolated root, or the suite would corrupt \
                 the real configuration of whoever runs it. Got: {}",
                env[key]
            );
        }
    }

    #[test]
    fn the_fake_directory_comes_first_on_path() {
        let outside = tempfile::tempdir().unwrap();
        let iso = Isolation::prepare(
            &case(),
            &fake_binary(outside.path()),
            &[],
            &[],
            outside.path(),
        )
        .unwrap();
        let env = env_of(&iso);

        let first = env["PATH"].split(':').next().unwrap().to_string();
        assert_eq!(
            first,
            iso.root().join("bin").to_string_lossy(),
            "the fake directory must be first, or the real binary would answer instead"
        );
        assert!(
            env["PATH"].split(':').count() > 1,
            "the rest of PATH must survive: passthrough needs the real binaries"
        );
    }

    #[test]
    fn each_faked_binary_gets_a_symlink_to_the_fake() {
        let outside = tempfile::tempdir().unwrap();
        let fake = fake_binary(outside.path());
        let bins = ["git".to_string(), "kubectl".to_string()];
        let iso = Isolation::prepare(&case(), &fake, &bins, &[], outside.path()).unwrap();

        for name in bins {
            let link = iso.root().join("bin").join(&name);
            assert!(link.exists(), "{name} was not laid down");
            assert_eq!(
                std::fs::read_link(&link).unwrap(),
                fake,
                "the symlink must point at the fake binary: that is how the fake \
                 discovers which tool it stands in for, through its argv[0]"
            );
        }
    }

    #[test]
    fn the_engine_variables_are_set_for_the_fake() {
        let outside = tempfile::tempdir().unwrap();
        let iso = Isolation::prepare(
            &case(),
            &fake_binary(outside.path()),
            &[],
            &[],
            outside.path(),
        )
        .unwrap();
        let env = env_of(&iso);

        for key in [
            gaveldrop_fake::env::SCENARIO,
            gaveldrop_fake::env::STATE,
            gaveldrop_fake::env::JOURNAL,
            gaveldrop_fake::env::DIR,
            gaveldrop_fake::env::CASE,
        ] {
            assert!(env.contains_key(key), "{key} must be set");
        }
        assert_eq!(env[gaveldrop_fake::env::CASE], "t");
        assert_eq!(
            env[gaveldrop_fake::env::JOURNAL],
            iso.journal_path().to_string_lossy()
        );
    }

    #[test]
    fn a_case_can_use_the_reserved_port_in_a_path_or_a_url() {
        let outside = tempfile::tempdir().unwrap();
        let iso = Isolation::prepare(
            &case(),
            &fake_binary(outside.path()),
            &[],
            &[],
            outside.path(),
        )
        .unwrap();

        let port = iso.defined()["GAVELDROP_PORT"].clone();
        assert!(
            port.parse::<u16>().is_ok(),
            "the port reaches a case through the same closed set of variables as HOME, which is \
             what lets it be written `$GAVELDROP_PORT` in a URL without the case depending on \
             anything its runner happens to have set. Got {port:?}"
        );
        assert!(
            std::net::TcpListener::bind(("127.0.0.1", port.parse::<u16>().unwrap_or(0))).is_ok(),
            "and nobody must be listening on it yet: the subject is the one that binds it"
        );
    }

    #[test]
    fn every_isolation_reserves_its_own_port() {
        let outside = tempfile::tempdir().unwrap();
        let fake = fake_binary(outside.path());
        let first = Isolation::prepare(&case(), &fake, &[], &[], outside.path()).unwrap();
        let held = std::net::TcpListener::bind((
            "127.0.0.1",
            first.defined()["GAVELDROP_PORT"].parse::<u16>().unwrap(),
        ))
        .unwrap();
        let second = Isolation::prepare(&case(), &fake, &[], &[], outside.path()).unwrap();
        drop(held);

        assert_ne!(
            first.defined()["GAVELDROP_PORT"],
            second.defined()["GAVELDROP_PORT"],
            "two cases must not be handed the same port while the first still holds it, or the \
             second subject fails to start for a reason belonging to neither case"
        );
    }

    #[test]
    fn the_scenario_is_written_even_when_the_case_has_no_fake_block() {
        let outside = tempfile::tempdir().unwrap();
        let iso = Isolation::prepare(
            &case(),
            &fake_binary(outside.path()),
            &[],
            &[],
            outside.path(),
        )
        .unwrap();
        let env = env_of(&iso);
        let scenario = PathBuf::from(&env[gaveldrop_fake::env::SCENARIO]);

        assert!(scenario.is_file(), "the fake must always find a scenario");
        let loaded = gaveldrop_fake::Scenario::load(&scenario).unwrap();
        assert!(
            loaded.validate().is_ok(),
            "a case with no `fake:` block still gets a catch-all-only scenario, so any \
             unexpected call is loud rather than a missing-file crash"
        );
    }

    #[test]
    fn variables_that_could_bypass_the_redirection_are_cleared_not_overridden() {
        let outside = tempfile::tempdir().unwrap();
        let cleared = ["MYTOOL_CONFIG_DIR".to_string()];
        let iso = Isolation::prepare(
            &case(),
            &fake_binary(outside.path()),
            &[],
            &cleared,
            outside.path(),
        )
        .unwrap();

        assert_eq!(iso.cleared(), cleared);
        assert!(
            !env_of(&iso).contains_key("MYTOOL_CONFIG_DIR"),
            "a variable that could short-circuit isolation must be removed, not given a \
             new value: a project reading it before looking at HOME would escape"
        );
    }

    #[test]
    fn two_isolations_never_share_a_root() {
        let outside = tempfile::tempdir().unwrap();
        let fake = fake_binary(outside.path());
        let first = Isolation::prepare(&case(), &fake, &[], &[], outside.path()).unwrap();
        let second = Isolation::prepare(&case(), &fake, &[], &[], outside.path()).unwrap();
        assert_ne!(first.root(), second.root());
    }

    #[test]
    fn every_redirected_directory_exists_before_the_subject_runs() {
        let outside = tempfile::tempdir().unwrap();
        let iso = Isolation::prepare(
            &case(),
            &fake_binary(outside.path()),
            &[],
            &[],
            outside.path(),
        )
        .unwrap();
        let env = env_of(&iso);

        for key in [
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
        ] {
            assert!(
                PathBuf::from(&env[key]).is_dir(),
                "{key} is pointed somewhere that does not exist; a subject that only \
                 writes into it would fail for a reason the case never asked about"
            );
        }
    }
}
