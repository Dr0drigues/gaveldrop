//! The project configuration: written once per repository, not per case.

use std::path::{Path, PathBuf};

use crate::Case;

use serde::{Deserialize, Serialize};

/// What a project tells gaveldrop about itself.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Glob pattern locating the case files, relative to the repository root.
    pub cases: String,
    /// How dependencies are faked.
    #[serde(default)]
    pub fake: FakeConfig,
    /// Environment variables to remove from every subject.
    ///
    /// List anything a project reads before it looks at the home directory. Such a
    /// variable would short-circuit isolation if it happened to be set in the environment
    /// of whoever runs the tests.
    #[serde(default)]
    pub clear_env: Vec<String>,
    /// How this project's structured events are recognised. Omit it when the subject emits
    /// none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<crate::verdict::events::EventsConfig>,
    /// The thresholds a run must meet, beyond every case simply passing.
    ///
    /// In the configuration rather than on the command line, deliberately: a bar that moves
    /// depending on who typed the command is not a bar.
    #[serde(default)]
    pub gate: GateConfig,
    /// Invariants this project names, each parameterising one of the four shapes.
    ///
    /// Named here and used by name in a case: that is what lets an invariant be written once
    /// and serve everywhere, without the core learning this project's event vocabulary.
    #[serde(default)]
    pub invariants: crate::verdict::invariants::NamedInvariants,
    /// How many seconds one case's subject may run before it is killed. `0` means no limit.
    ///
    /// Omitted, [`DEFAULT_TIMEOUT_SECONDS`] applies. **A default rather than opt-in**, because the
    /// thing it prevents costs hours rather than minutes: a subject that never returns used to hang
    /// the case, the suite and the continuous-integration job behind it until whatever global limit
    /// the runner had. A guard nobody had to read about is the only kind that helps there.
    ///
    /// Generous on purpose. It is a guard against a hang, not a performance threshold — that would be
    /// a number a loaded machine can trip, and this project reports durations precisely because it
    /// refuses to gate on them.
    ///
    /// A single case that legitimately takes longer overrides it with its own `timeout:`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

/// How long a case's subject may run when nothing says otherwise.
///
/// Five minutes: far above anything a real case does — the slowest here is half a second, and a
/// consumer's whole suite is a few seconds — and far below the global limit of any CI runner, so the
/// job fails with a verdict naming the case instead of being cut off with nothing.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 300;

/// What a run must clear to be considered a pass.
///
/// Every knob is optional and absent means unenforced. Gating is opt-in because adding it must not
/// start failing projects that never asked for a threshold — a failing case already fails the run on
/// its own, which is a different question from whether the suite met a bar.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GateConfig {
    /// The least weighted score that counts as a pass.
    ///
    /// Answers "how much of the weight has to hold", which is the question a project with a long
    /// tail of low-weight cases actually cares about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_score: Option<u32>,
    /// How many `allow_fail` cases may fail before the exemption is a lie.
    ///
    /// An exemption nobody counts becomes a habit, and a suite where half the cases are tolerated is
    /// green for no reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tolerated: Option<usize>,
    /// A weight above which a single failing case fails the run on its own.
    ///
    /// Ninety percent of the weight holding is no comfort when the case that broke is the one that
    /// mattered. A **tolerated** failure does not trip this: a declared exemption is not a surprise,
    /// and `max_tolerated` is the knob that counts those.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_above_weight: Option<u32>,
}

/// How dependencies are faked.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FakeConfig {
    /// Refuse to let any rule reach the real tool, wherever this runs.
    ///
    /// For an environment where the real tool cannot work — CI with no credentials and no network. A
    /// rule that passes through must then declare what it answers instead, or the run is refused
    /// rather than answered with silence.
    ///
    /// Usually set from the environment in the CI job rather than committed, so a laptop keeps
    /// reaching the real tool.
    #[serde(default)]
    pub no_passthrough: bool,
    /// The binaries to shadow. Each gets a symlink to the fake, first on `PATH`.
    #[serde(default)]
    pub bins: Vec<String>,
}

/// What can go wrong with the configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The configuration file could not be read.
    #[error("reading the configuration {path}: {source}")]
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The configuration is not valid YAML, or has an unknown key.
    #[error("configuration {path} is invalid: {source}")]
    Parse {
        /// The offending path.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: serde_yaml_ng::Error,
    },
    /// The `cases` pattern is not a valid glob.
    #[error("the `cases` pattern {pattern:?} is not a valid glob: {source}")]
    Pattern {
        /// The offending pattern.
        pattern: String,
        /// The underlying failure.
        #[source]
        source: glob::PatternError,
    },
    /// A shard was asked for that does not exist.
    ///
    /// Loud rather than empty: `--shard 4/3` is a typo in a CI matrix, and a silent empty run
    /// reports success, which is the worst possible answer to it.
    #[error(
        "there is no shard {index} of {of}. Shards are 0-indexed, so the last one is {}",
        of.saturating_sub(1)
    )]
    ShardOutOfRange {
        /// The index asked for.
        index: usize,
        /// How many shards were declared.
        of: usize,
    },
    /// A selection fragment matched no case.
    #[error(
        "no case path contains {}. A filter that matched nothing would otherwise run \
         zero cases and report success",
        fragments.iter().map(|f| format!("{f:?}")).collect::<Vec<_>>().join(" or ")
    )]
    NothingSelected {
        /// Every fragment nobody matched.
        ///
        /// All of them, not the first. Repeating `--only` is for running four cases at once, and
        /// discovering the typo in the fourth after fixing the second is two runs where one would do
        /// — the same reason `gate()` reports every reason it failed.
        fragments: Vec<String>,
    },
    /// The pattern matched no file.
    #[error(
        "the `cases` pattern {pattern:?} matched no file under {root}. A suite with no \
         cases would pass while proving nothing"
    )]
    NoCases {
        /// The pattern that found nothing.
        pattern: String,
        /// Where it was resolved from.
        root: PathBuf,
    },
    /// Two or more cases claim the same `name:`.
    #[error(
        "{}. A name is what identifies a case in every report this project writes — a JUnit file \
         with two testcases of the same name is malformed for several dashboards, the HTML report \
         keys each case's detail by it, and a terminal line naming a failure would not say which \
         file to open. Rename one",
        collisions.iter().map(|(name, files)| format!(
            "{} cases are called {name:?}: {}", files.len(), listed(files)
        )).collect::<Vec<_>>().join("; ")
    )]
    DuplicateNames {
        /// Every repeated name, with the files claiming it.
        ///
        /// All of them at once, like every other report of several problems here: renaming one and
        /// rerunning to discover the next is as many runs as there are collisions.
        collisions: Vec<(String, Vec<String>)>,
    },
}

/// A list of files as a sentence reads it: `a and b`, or `a, b and c`.
///
/// Small, and it earns its place: this message is the whole deliverable of the check above, and
/// `a and b and c` is the kind of wording that makes a reader wonder whether the tool is finished.
fn listed(files: &[String]) -> String {
    match files.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// Every name claimed by more than one case, with the files claiming it.
///
/// Checked before any case is prepared, so a suite that cannot be reported on does not half-run
/// first. It is a suite-level mistake rather than a case failure: making the second of two
/// same-named cases fail would still leave two identically named entries in the report, which is
/// the thing that cannot be read.
pub fn duplicate_names(named: &[(String, String)]) -> Vec<(String, Vec<String>)> {
    let mut by_name: std::collections::BTreeMap<&str, Vec<String>> =
        std::collections::BTreeMap::new();
    for (name, file) in named {
        by_name.entry(name).or_default().push(file.clone());
    }

    by_name
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(name, files)| (name.to_string(), files))
        .collect()
}

impl Config {
    /// Loads the configuration from `path`.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        serde_yaml_ng::from_str(&raw).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Expands the `cases` pattern, sorted.
    ///
    /// Sorted because an unstable order makes a report unstable, and a sharded run
    /// unreproducible. Empty is an error, not a vacuous pass.
    pub fn discover(&self, root: &Path) -> Result<Vec<PathBuf>, ConfigError> {
        let pattern = root.join(&self.cases).to_string_lossy().into_owned();
        let matches = glob::glob(&pattern).map_err(|source| ConfigError::Pattern {
            pattern: self.cases.clone(),
            source,
        })?;

        let mut found: Vec<PathBuf> = matches.filter_map(Result::ok).collect();
        found.sort();

        if found.is_empty() {
            return Err(ConfigError::NoCases {
                pattern: self.cases.clone(),
                root: root.to_path_buf(),
            });
        }

        Ok(found)
    }
}

/// Which slice of the suite this run is responsible for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shard {
    /// This runner's position, 0-indexed.
    pub index: usize,
    /// How many runners there are in total.
    pub of: usize,
}

/// How long `case` may run under `config`, or `None` when neither sets a limit.
///
/// The case wins over the project, and a `0` on either means no limit — the escape hatch for a suite
/// whose subject legitimately runs for as long as it needs. Resolved here rather than in the runner
/// so the rule has one home: an adapter reads the answer off the isolation and never has to know that
/// two places could have set it.
pub fn limit_for(config: &Config, case: &Case) -> Option<std::time::Duration> {
    match case.timeout.or(config.timeout) {
        Some(0) => None,
        Some(seconds) => Some(std::time::Duration::from_secs(seconds)),
        None => Some(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECONDS)),
    }
}

/// The cases this run should take, from everything that was discovered.
///
/// **The filter applies before the shard**, so a partition is of what was asked for. Sharding first
/// and filtering after would leave some runners with nothing and make the partition meaningless.
///
/// Sharding is `index modulo of` — **interleaved, not contiguous.** Contiguous blocks put the slow
/// cases on one runner, because cases that sort together usually share a prefix and therefore a
/// subject. Discovery is already sorted, so the partition is identical on every machine without a
/// coordinator, a manifest or a lock.
pub fn select(
    discovered: Vec<PathBuf>,
    shard: Option<Shard>,
    only: &[String],
) -> Result<Vec<PathBuf>, ConfigError> {
    let kept = if only.is_empty() {
        discovered
    } else {
        // Every fragment has to match something, checked before anything is kept. The rule it comes
        // from — a filter matching nothing is an error rather than a green run — applies to each
        // fragment on its own, or a typo in one would be absorbed by another one's matches and the
        // run would look like it did what was asked.
        let unmatched: Vec<String> = only
            .iter()
            .filter(|fragment| !discovered.iter().any(|path| holds(path, fragment)))
            .cloned()
            .collect();

        if !unmatched.is_empty() {
            return Err(ConfigError::NothingSelected {
                fragments: unmatched,
            });
        }

        // A union, which is what repeating a flag means everywhere else. Order stays discovery's, so
        // the report reads the same however the fragments were typed.
        discovered
            .into_iter()
            .filter(|path| only.iter().any(|fragment| holds(path, fragment)))
            .collect()
    };

    let Some(shard) = shard else {
        return Ok(kept);
    };

    if shard.of == 0 || shard.index >= shard.of {
        return Err(ConfigError::ShardOutOfRange {
            index: shard.index,
            of: shard.of,
        });
    }

    Ok(kept
        .into_iter()
        .enumerate()
        .filter(|(at, _)| at % shard.of == shard.index)
        .map(|(_, path)| path)
        .collect())
}

/// Whether a case's path contains `fragment`.
///
/// One function so the check that decides what runs and the check that decides what is an error
/// cannot drift. Two spellings of "matches" is how a fragment ends up rejected as unmatched and then
/// silently kept, or the reverse.
fn holds(path: &Path, fragment: &str) -> bool {
    path.to_string_lossy().contains(fragment)
}

#[cfg(test)]
mod tests {

    fn only(fragments: &[&str]) -> Vec<String> {
        fragments.iter().map(|f| f.to_string()).collect()
    }

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    /// The case wins over the project, and `0` on either means no limit.
    #[test]
    fn a_case_can_raise_or_remove_the_projects_limit() {
        let timed = |project: Option<u64>, case: Option<u64>| {
            let config = Config {
                timeout: project,
                ..Default::default()
            };
            let subject = Case {
                timeout: case,
                ..Default::default()
            };
            limit_for(&config, &subject).map(|limit| limit.as_secs())
        };

        assert_eq!(
            timed(None, None),
            Some(DEFAULT_TIMEOUT_SECONDS),
            "a project that says nothing is still guarded, or the guard protects only the people who \
             read about it"
        );
        assert_eq!(timed(Some(30), None), Some(30), "the project sets it");
        assert_eq!(
            timed(Some(30), Some(900)),
            Some(900),
            "and the one case that legitimately takes longer raises it for itself, rather than \
             forcing the project to raise it for everyone"
        );
        assert_eq!(timed(Some(0), None), None, "zero is the escape hatch");
        assert_eq!(
            timed(Some(30), Some(0)),
            None,
            "and a case can take the escape hatch on its own"
        );
    }

    /// Two cases claiming one name are named, with both files.
    #[test]
    fn a_repeated_name_is_reported_with_every_file_claiming_it() {
        let named = vec![
            ("dupe".to_string(), "cases/a.yaml".to_string()),
            ("fine".to_string(), "cases/b.yaml".to_string()),
            ("dupe".to_string(), "cases/c.yaml".to_string()),
        ];

        let collisions = duplicate_names(&named);

        assert_eq!(
            collisions,
            vec![(
                "dupe".to_string(),
                vec!["cases/a.yaml".to_string(), "cases/c.yaml".to_string()]
            )],
            "both files, because the fix is to open one of them"
        );
    }

    /// Every collision at once.
    #[test]
    fn every_repeated_name_is_reported_together() {
        let named = vec![
            ("a".to_string(), "1.yaml".to_string()),
            ("a".to_string(), "2.yaml".to_string()),
            ("b".to_string(), "3.yaml".to_string()),
            ("b".to_string(), "4.yaml".to_string()),
        ];

        assert_eq!(
            duplicate_names(&named).len(),
            2,
            "renaming one and rerunning to find the next is as many runs as there are collisions"
        );
    }

    #[test]
    fn a_suite_where_every_name_is_distinct_has_no_collisions() {
        let named = vec![
            ("a".to_string(), "1.yaml".to_string()),
            ("b".to_string(), "2.yaml".to_string()),
        ];

        assert!(duplicate_names(&named).is_empty());
    }

    /// The message is the deliverable, so it reads as a sentence.
    #[test]
    fn a_list_of_files_reads_as_a_sentence() {
        assert_eq!(listed(&["a".to_string()]), "a");
        assert_eq!(listed(&["a".to_string(), "b".to_string()]), "a and b");
        assert_eq!(
            listed(&["a".to_string(), "b".to_string(), "c".to_string()]),
            "a, b and c",
            "`a and b and c` is the wording that makes a reader wonder whether the tool is finished"
        );
    }

    #[test]
    fn every_case_lands_in_exactly_one_shard() {
        let all = paths(&["a", "b", "c", "d", "e", "f", "g"]);
        let mut seen: Vec<PathBuf> = Vec::new();

        for index in 0..3 {
            let shard = Shard { index, of: 3 };
            seen.extend(select(all.clone(), Some(shard), &[]).unwrap());
        }
        seen.sort();

        assert_eq!(
            seen, all,
            "a partition, not a sample. A case in two shards is counted twice in the merged \
             report; a case in none is silently untested, which is the worse of the two"
        );
    }

    #[test]
    fn shards_are_interleaved_rather_than_contiguous() {
        let all = paths(&["a", "b", "c", "d", "e", "f"]);
        let first = select(all, Some(Shard { index: 0, of: 3 }), &[]).unwrap();

        assert_eq!(
            first,
            paths(&["a", "d"]),
            "contiguous blocks put the slow cases on one runner, because cases that sort together \
             usually share a prefix and a subject"
        );
    }

    #[test]
    fn one_shard_of_one_is_every_case() {
        let all = paths(&["a", "b", "c"]);

        assert_eq!(
            select(all.clone(), Some(Shard { index: 0, of: 1 }), &[]).unwrap(),
            all,
            "the default path must not be a special case in the code"
        );
    }

    #[test]
    fn no_shard_at_all_is_every_case() {
        let all = paths(&["a", "b"]);
        assert_eq!(select(all.clone(), None, &[]).unwrap(), all);
    }

    #[test]
    fn a_shard_index_outside_the_range_is_an_error_naming_the_range() {
        let error = select(paths(&["a"]), Some(Shard { index: 3, of: 3 }), &[]).unwrap_err();

        let said = error.to_string();
        assert!(
            said.contains('3'),
            "`--shard 4/3` is a typo in a CI matrix, and a silent empty run would look like a \
             passing one: {said}"
        );
    }

    #[test]
    fn a_shard_of_zero_is_an_error_rather_than_a_division() {
        assert!(select(paths(&["a"]), Some(Shard { index: 0, of: 0 }), &[]).is_err());
    }

    #[test]
    fn only_keeps_the_cases_whose_path_contains_the_fragment() {
        let all = paths(&["tests/cases/an-order.yaml", "tests/cases/a-service.yaml"]);

        assert_eq!(
            select(all, None, &only(&["order"])).unwrap(),
            paths(&["tests/cases/an-order.yaml"])
        );
    }

    #[test]
    fn only_matching_nothing_is_an_error_naming_the_fragment() {
        let error = select(paths(&["a.yaml"]), None, &only(&["nowhere"])).unwrap_err();

        assert!(
            error.to_string().contains("nowhere"),
            "a filter that matched nothing must say so: an empty run reports success, which is the \
             worst possible answer to a mistyped filter: {}",
            error
        );
    }

    /// Several fragments are a union, which is what repeating a flag means everywhere else.
    #[test]
    fn several_fragments_keep_every_case_any_of_them_matches() {
        let all = paths(&["cases/login.yaml", "cases/order.yaml", "cases/logout.yaml"]);

        assert_eq!(
            select(all, None, &only(&["login", "logout"])).unwrap(),
            paths(&["cases/login.yaml", "cases/logout.yaml"]),
            "and in discovery's order, so the report reads the same however the fragments were typed"
        );
    }

    /// A typo in one fragment cannot be absorbed by another one's matches.
    ///
    /// This is the whole reason the check is per fragment. `--only login --only lgout` would
    /// otherwise run the login cases and report success, having silently done half of what was
    /// asked — which is exactly the failure the single-fragment rule was written to prevent.
    #[test]
    fn a_fragment_matching_nothing_is_an_error_even_when_another_matched() {
        let all = paths(&["cases/login.yaml", "cases/order.yaml"]);
        let error = select(all, None, &only(&["login", "lgout"])).unwrap_err();

        assert!(
            error.to_string().contains("lgout"),
            "the typo has to be named, not merely counted: {error}"
        );
    }

    /// Every unmatched fragment, not the first.
    #[test]
    fn every_fragment_that_matched_nothing_is_named_at_once() {
        let error = select(paths(&["a.yaml"]), None, &only(&["nope", "nor-this"])).unwrap_err();
        let said = error.to_string();

        assert!(
            said.contains("nope") && said.contains("nor-this"),
            "fixing one and rerunning to discover the other is two runs where one would do: {said}"
        );
    }

    #[test]
    fn a_filter_applies_before_the_shard_so_the_partition_is_of_what_was_asked_for() {
        let all = paths(&["keep-a", "drop-1", "keep-b", "drop-2"]);
        let first = select(
            all.clone(),
            Some(Shard { index: 0, of: 2 }),
            &only(&["keep"]),
        )
        .unwrap();
        let second = select(all, Some(Shard { index: 1, of: 2 }), &only(&["keep"])).unwrap();

        assert_eq!(first, paths(&["keep-a"]));
        assert_eq!(
            second,
            paths(&["keep-b"]),
            "sharding what was filtered, not filtering what was sharded — otherwise a matrix with a \
             filter leaves some runners with nothing and the partition means nothing"
        );
    }
    use super::*;

    #[test]
    fn a_config_parses_and_defaults_the_optional_blocks() {
        let config: Config = serde_yaml_ng::from_str("cases: tests/cases/**/*.yaml\n").unwrap();

        assert_eq!(config.cases, "tests/cases/**/*.yaml");
        assert!(config.fake.bins.is_empty());
        assert!(config.clear_env.is_empty());
    }

    #[test]
    fn a_full_config_parses() {
        let yaml = r#"
cases: tests/cases/**/*.yaml
fake:
  bins: [git, kubectl]
clear_env: [MYTOOL_CONFIG_DIR]
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();

        assert_eq!(config.fake.bins, vec!["git", "kubectl"]);
        assert_eq!(config.clear_env, vec!["MYTOOL_CONFIG_DIR"]);
    }

    #[test]
    fn a_typo_in_the_config_is_refused() {
        let error = serde_yaml_ng::from_str::<Config>("casez: x\n").unwrap_err();
        assert!(error.to_string().contains("casez") || error.to_string().contains("cases"));
    }

    #[test]
    fn discovery_finds_cases_in_a_stable_order() {
        let dir = tempfile::tempdir().unwrap();
        let cases = dir.path().join("tests/cases");
        std::fs::create_dir_all(&cases).unwrap();
        for name in ["zebra.yaml", "alpha.yaml", "middle.yaml", "ignored.txt"] {
            std::fs::write(cases.join(name), "").unwrap();
        }

        let config = Config {
            cases: "tests/cases/**/*.yaml".to_string(),
            ..Default::default()
        };
        let names: Vec<String> = config
            .discover(dir.path())
            .unwrap()
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();

        assert_eq!(
            names,
            vec!["alpha.yaml", "middle.yaml", "zebra.yaml"],
            "discovery must be sorted: an unstable order makes a report unstable, and a \
             sharded run unreproducible"
        );
    }

    #[test]
    fn a_pattern_that_matches_nothing_is_an_error_not_a_vacuous_pass() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            cases: "tests/cases/**/*.yaml".to_string(),
            ..Default::default()
        };

        let error = config.discover(dir.path()).unwrap_err();
        assert!(
            error.to_string().contains("tests/cases"),
            "a suite with no cases would pass while proving nothing; say so and name the \
             pattern: {error}"
        );
    }

    #[test]
    fn named_invariants_parse_alongside_the_events_block() {
        let yaml = r#"
cases: tests/cases/**/*.yaml
events:
  type_field: t
invariants:
  agent_start_end_symmetric: { shape: paired, start: agent_start, end: agent_end, key: agent }
  single_result:             { shape: exactly_one, type: result }
  prov_model_non_empty:      { shape: field_non_empty, type: provider, field: model }
  no_orphan_events:          { shape: no_orphan, key: agent, root: agent_start }
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();

        assert_eq!(config.invariants.len(), 4);
        assert_eq!(config.events.as_ref().unwrap().type_field, "t");
    }

    #[test]
    fn a_config_without_invariants_gets_an_empty_set_rather_than_failing() {
        let config: Config = serde_yaml_ng::from_str("cases: x\n").unwrap();
        assert!(config.invariants.is_empty());
    }

    #[test]
    fn a_config_loads_from_a_file_and_the_message_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gaveldrop.yaml");
        std::fs::write(&path, "cases: tests/cases/**/*.yaml\n").unwrap();
        assert_eq!(Config::load(&path).unwrap().cases, "tests/cases/**/*.yaml");

        let error = Config::load(&dir.path().join("absent.yaml")).unwrap_err();
        assert!(
            error.to_string().contains("absent.yaml"),
            "an error message must name the offender: {error}"
        );
    }
}
