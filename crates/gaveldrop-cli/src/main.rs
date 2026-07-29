//! The command-line facade.
//!
//! It contains **no logic**. Everything it does is available from the library: a behaviour
//! that only exists by going through this binary is a behaviour a Rust project cannot test.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use gaveldrop::report::terminal::Terminal;
use gaveldrop::{Config, runner};

/// Run YAML-driven test cases.
#[derive(Debug, Parser)]
#[command(name = "gaveldrop", version, about)]
struct Cli {
    /// Path to the project configuration.
    #[arg(long, default_value = "gaveldrop.yaml")]
    config: PathBuf,
    /// Repository root the `cases` pattern resolves from. Defaults to the configuration's
    /// own directory, so running from a subdirectory behaves the same.
    #[arg(long)]
    root: Option<PathBuf>,
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("gaveldrop: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the suite. `Ok(false)` means it ran and something failed, which is a verdict rather
/// than an error — so it must not be reported as one.
fn run() -> Result<bool> {
    let cli = Cli::parse();

    let config = Config::load(&cli.config).with_context(|| {
        format!(
            "no usable configuration at {}. Create a `gaveldrop.yaml` with at least a \
             `cases:` pattern",
            cli.config.display()
        )
    })?;

    let root = cli
        .root
        .clone()
        .or_else(|| resolvable_parent(&cli.config))
        .unwrap_or_else(|| PathBuf::from("."));

    let fake_binary = locate_fake().context(
        "the fake binary was not found beside this executable, and it is what shadows the \
         dependencies a case fakes",
    )?;

    let mut sink = Terminal::styled(anstream::stdout());
    let report = runner::run_all(&config, &root, &fake_binary, &mut sink)?;

    Ok(report.is_success())
}

/// The directory holding `config`, when it has one worth using.
///
/// A bare `gaveldrop.yaml` has an empty parent, which would resolve the cases pattern
/// against the filesystem root rather than the working directory.
fn resolvable_parent(config: &Path) -> Option<PathBuf> {
    config
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(PathBuf::from)
}

/// Finds `gaveldrop-fake` beside this executable.
///
/// Beside, rather than on `PATH`: the two ship together and must stay in step. Picking up a
/// different version from `PATH` would mean a scenario shape mismatch surfacing as an
/// unexplained case failure.
fn locate_fake() -> Result<PathBuf> {
    let here = std::env::current_exe()?;
    here.parent()
        .map(|dir| dir.join("gaveldrop-fake"))
        .filter(|path| path.is_file())
        .context("no `gaveldrop-fake` beside the running executable")
}
