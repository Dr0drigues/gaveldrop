//! The project configuration: written once per repository, not per case.

use std::path::{Path, PathBuf};

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
}

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

#[cfg(test)]
mod tests {
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
