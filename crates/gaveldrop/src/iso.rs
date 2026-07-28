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

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use gaveldrop_fake::{Match, Response, Rule, Scenario};

use crate::Case;

/// A pristine directory, the environment that points into it, and the symlinks that put
/// the fake first on the search path.
pub struct Isolation {
    root: tempfile::TempDir,
    env: Vec<(String, OsString)>,
    cleared: Vec<String>,
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
}

impl Isolation {
    /// Prepares everything a case needs to run, and nothing it could escape through.
    ///
    /// `faked_bins` names the tools to shadow; each gets a symlink to `fake_binary` in a
    /// directory placed first on `PATH`. `clear_env` names the variables to remove — see
    /// [`Isolation::cleared`] for why removing beats overriding.
    pub fn prepare(
        case: &Case,
        fake_binary: &Path,
        faked_bins: &[String],
        clear_env: &[String],
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
            let link = bin_dir.join(name);
            std::os::unix::fs::symlink(fake_binary, &link)
                .map_err(|source| IsoError::Io { path: link, source })?;
        }

        let scenario_path = base.join("scenario.yaml");
        write_scenario(&scenario_path, case.fake.as_ref())?;

        let journal = base.join("journal.jsonl");
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let mut path = OsString::from(&bin_dir);
        path.push(":");
        path.push(&inherited);

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
        ];

        Ok(Self {
            root,
            env,
            cleared: clear_env.to_vec(),
        })
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
fn write_scenario(path: &Path, declared: Option<&Scenario>) -> Result<(), IsoError> {
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

    let yaml = serde_yaml_ng::to_string(declared.unwrap_or(&fallback))?;
    std::fs::write(path, yaml).map_err(|source| IsoError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
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
        let iso = Isolation::prepare(&case(), &fake_binary(outside.path()), &[], &[]).unwrap();
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
        let iso = Isolation::prepare(&case(), &fake_binary(outside.path()), &[], &[]).unwrap();
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
        let iso = Isolation::prepare(&case(), &fake, &bins, &[]).unwrap();

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
        let iso = Isolation::prepare(&case(), &fake_binary(outside.path()), &[], &[]).unwrap();
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
    fn the_scenario_is_written_even_when_the_case_has_no_fake_block() {
        let outside = tempfile::tempdir().unwrap();
        let iso = Isolation::prepare(&case(), &fake_binary(outside.path()), &[], &[]).unwrap();
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
        let iso = Isolation::prepare(&case(), &fake_binary(outside.path()), &[], &cleared).unwrap();

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
        let first = Isolation::prepare(&case(), &fake, &[], &[]).unwrap();
        let second = Isolation::prepare(&case(), &fake, &[], &[]).unwrap();
        assert_ne!(first.root(), second.root());
    }

    #[test]
    fn every_redirected_directory_exists_before_the_subject_runs() {
        let outside = tempfile::tempdir().unwrap();
        let iso = Isolation::prepare(&case(), &fake_binary(outside.path()), &[], &[]).unwrap();
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
