//! The command-line facade.
//!
//! It contains **no logic**. Everything it does is available from the library: a behaviour
//! that only exists by going through this binary is a behaviour a Rust project cannot test.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use gaveldrop::config::Shard;
use gaveldrop::report::annotate::Annotate;
use gaveldrop::report::html::Html;
use gaveldrop::report::jsonl::Jsonl;
use gaveldrop::report::junit::Junit;
use gaveldrop::report::terminal::Terminal;
use gaveldrop::report::verbose::Verbose;
use gaveldrop::{Config, Tee, runner, watch};

/// Run YAML-driven test cases.
#[derive(Debug, Parser)]
#[command(name = "gaveldrop", version, about)]
struct Cli {
    /// Path to the project configuration.
    #[arg(long, default_value = "gaveldrop.yaml")]
    config: PathBuf,
    /// Write a machine-readable report here, one JSON object per case.
    #[arg(long, value_name = "PATH")]
    report_json: Option<PathBuf>,
    /// Write a self-contained HTML report here.
    #[arg(long, value_name = "PATH")]
    report_html: Option<PathBuf>,
    /// Write a JUnit XML report here, for a CI dashboard to read.
    #[arg(long, value_name = "PATH")]
    report_junit: Option<PathBuf>,
    /// Emit GitHub workflow commands on standard output, one per failing case.
    #[arg(long)]
    annotate: bool,
    /// Print what the engine decided for each case before running it: the adapter, the isolated
    /// root, the tools faked and hidden, the variables the case declared.
    #[arg(long, short)]
    verbose: bool,
    /// Run only this slice of the suite, as `N/M` with N 0-indexed.
    #[arg(long, value_name = "N/M")]
    shard: Option<String>,
    /// Run only the cases whose path contains this fragment.
    #[arg(long, value_name = "FRAGMENT")]
    only: Option<String>,
    /// List the cases as JSON and run nothing, for an editor's test interface.
    #[arg(long)]
    list: bool,
    /// Keep running, re-running what a save affected. Ends on Ctrl-C.
    #[arg(long)]
    watch: bool,
    /// Extra paths to watch besides the cases: a directory of shell functions, a service.
    #[arg(long, value_name = "PATH")]
    watch_also: Vec<PathBuf>,
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

    if cli.list {
        let discovered = gaveldrop::inspect(&gaveldrop::config::select(
            config.discover(&root)?,
            parse_shard(cli.shard.as_deref())?,
            cli.only.as_deref(),
        )?);
        println!("{}", serde_json::to_string_pretty(&discovered)?);

        // Listing is not a verdict. An editor drawing a tree must not be told the suite failed
        // because it has not run yet, and a broken document is reported inside the listing rather
        // than as an exit code.
        return Ok(true);
    }

    // No `.context` here: the library's message already names where it looked and what to run,
    // and a wrapper repeating "beside this executable" would now contradict it — PATH is searched
    // too.
    let fake_binary = locate_fake()?;

    let mut sink = Tee::new();
    if cli.verbose {
        sink.add(Box::new(Verbose::new(anstream::stdout())));
    }
    sink.add(Box::new(Terminal::styled(anstream::stdout())));
    if let Some(path) = &cli.report_json {
        sink.add(Box::new(Jsonl::new(create_report(path)?)));
    }
    if let Some(path) = &cli.report_html {
        sink.add(Box::new(Html::new(create_report(path)?)));
    }
    if let Some(path) = &cli.report_junit {
        sink.add(Box::new(Junit::new(create_report(path)?)));
    }
    let discovered = config.discover(&root).unwrap_or_default();
    if cli.annotate {
        sink.add(Box::new(Annotate::new(std::io::stdout(), &discovered)));
    }

    let report = runner::run_all_selected(
        &config,
        &root,
        &fake_binary,
        &mut sink,
        parse_shard(cli.shard.as_deref())?,
        cli.only.as_deref(),
    )?;

    let gating = report.gate(&config.gate);
    for reason in &gating.reasons {
        eprintln!("gaveldrop: {reason}");
    }

    if cli.watch {
        return keep_watching(&cli, &config, &root, &fake_binary, discovered);
    }

    // One exit code for both, on purpose. A caller asking "did this pass" wants one answer, and a
    // run that met every assertion but missed the project's bar did not pass.
    Ok(report.is_success() && gating.passed)
}

/// Reruns what a save affected, until Ctrl-C.
///
/// The first run has already happened when this is called: a watch that showed nothing until the first
/// save would leave you wondering whether it started.
///
/// It always reports success. A watch is a conversation, not a verdict — the exit code of a session you
/// ended with Ctrl-C says nothing useful, and a non-zero one would make `gaveldrop --watch` unusable in
/// anything that checks exit codes.
fn keep_watching(
    cli: &Cli,
    config: &Config,
    root: &Path,
    fake_binary: &Path,
    cases: Vec<PathBuf>,
) -> Result<bool> {
    let mut watched = cases.clone();
    watched.extend(cli.watch_also.iter().cloned());
    watched.push(cli.config.clone());

    eprintln!(
        "gaveldrop: watching {} files. Ctrl-C to stop.",
        watched.len()
    );
    let mut before = watch::Fingerprints::take(&watched);

    loop {
        std::thread::sleep(POLL);
        let now = watch::Fingerprints::take(&watched);
        let changed = now.changed_since(&before);
        before = now;

        let only = match watch::affected(&changed, &cases) {
            watch::Scope::Nothing => continue,
            watch::Scope::Everything => None,
            watch::Scope::Cases(touched) => touched
                .first()
                .and_then(|path| path.file_stem())
                .map(|stem| stem.to_string_lossy().into_owned()),
        };

        match &only {
            Some(name) => eprintln!("\ngaveldrop: {name} changed"),
            None => eprintln!("\ngaveldrop: something a case depends on changed, running all"),
        }

        let mut sink = Tee::new();
        // Same composition as the first run: a `--watch --verbose` session that stopped being
        // verbose after the first pass would be worse than one that never was.
        if cli.verbose {
            sink.add(Box::new(Verbose::new(anstream::stdout())));
        }
        sink.add(Box::new(Terminal::styled(anstream::stdout())));
        let _ =
            runner::run_all_selected(config, root, fake_binary, &mut sink, None, only.as_deref());
    }
}

/// How often to look. Long enough to be free, short enough to feel immediate.
const POLL: std::time::Duration = std::time::Duration::from_millis(300);

/// Reads `--shard N/M`.
///
/// Rejected here rather than clamped: `--shard 1` or `--shard 2/` is a typo in a CI matrix, and
/// guessing what was meant would run the wrong slice silently.
fn parse_shard(text: Option<&str>) -> Result<Option<Shard>> {
    let Some(text) = text else {
        return Ok(None);
    };

    let (index, of) = text
        .split_once('/')
        .with_context(|| format!("--shard wants `N/M`, and {text:?} has no `/`"))?;

    Ok(Some(Shard {
        index: index
            .trim()
            .parse()
            .with_context(|| format!("--shard index {index:?} is not a number"))?,
        of: of
            .trim()
            .parse()
            .with_context(|| format!("--shard count {of:?} is not a number"))?,
    }))
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

/// Opens a report file, creating parent directories.
///
/// Done **before** the suite runs. Failing afterwards would waste the whole run and report
/// nothing, which is the least useful moment to discover a bad path.
fn create_report(path: &Path) -> Result<std::fs::File> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating the report directory {}", parent.display()))?;
    }

    std::fs::File::create(path)
        .with_context(|| format!("creating the report file {}", path.display()))
}

/// Finds `gaveldrop-fake` beside this executable.
///
/// Beside, rather than on `PATH`: the two ship together and must stay in step. Picking up a
/// different version from `PATH` would mean a scenario shape mismatch surfacing as an
/// unexplained case failure.
fn locate_fake() -> Result<PathBuf> {
    if let Some(found) = gaveldrop::locate::fake_for_current_exe() {
        return Ok(found.path().to_path_buf());
    }

    let exe = std::env::current_exe()?;
    let dir = exe.parent().unwrap_or(Path::new(".")).to_path_buf();
    Err(anyhow::anyhow!(gaveldrop::locate::advice(&dir)))
}
