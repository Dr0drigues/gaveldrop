//! The driver: for each case, isolate, invoke, evaluate, report.

use std::path::{Path, PathBuf};

use crate::adapters::{self, Adapter};
use crate::config::ConfigError;
use crate::hooks;
use crate::report::Sink;
use crate::verdict::events::{self, Event};
use crate::verdict::{Context, evaluate_in};
use crate::{Case, Config, Diff, Isolation, Observations, Outcome, Report};

/// Runs every case the configuration finds, feeding `sink` as each one finishes.
///
/// Never panics on a broken case. A temporary directory that refuses to be created, a
/// program that will not start, a case that will not parse — each becomes a failed outcome
/// with a diagnostic, not a panic that takes the other ninety-nine with it.
///
/// The distinction that decides which is which: a **load** error is loud and stops
/// everything, because a suite that cannot find its cases must not report success. An
/// **execution** error is a case failure like any other.
pub fn run_all(
    config: &Config,
    root: &Path,
    fake_binary: &Path,
    sink: &mut dyn Sink,
) -> Result<Report, ConfigError> {
    run_all_selected(config, root, fake_binary, sink, None, &[])
}

/// Runs the slice of the suite this machine is responsible for, with the built-in adapters.
///
/// Selection happens before anything is prepared, so a shard that will not resolve fails before a
/// single case has run.
///
/// An empty `only` runs everything. Several fragments are a union, and every one of them has to match
/// something — a fragment matching nothing is an error, or a typo would be absorbed by its
/// neighbours' matches and the run would look like it did what was asked.
pub fn run_all_selected(
    config: &Config,
    root: &Path,
    fake_binary: &Path,
    sink: &mut dyn Sink,
    shard: Option<crate::config::Shard>,
    only: &[String],
) -> Result<Report, ConfigError> {
    run_all_with(
        config,
        root,
        fake_binary,
        sink,
        shard,
        only,
        &adapters::registry(),
    )
}

/// Runs the suite with adapters the caller supplies, rather than the built-in ones.
///
/// This is how a project whose cases carry vocabulary no built-in claims runs its own suite. Its
/// adapter is proved by the conformance kit, which has always taken one; until this function
/// existed the kit could prove an adapter that nothing was then able to use.
///
/// `adapters` is searched in order, so an adapter placed before the built-ins claims a case they
/// would also have claimed. To keep them, extend rather than replace.
///
/// The `gaveldrop` **binary** cannot reach an adapter compiled into someone else's crate, so a
/// project that writes one runs its suite from a Rust test calling this. Everything else is
/// unchanged: the same sinks, the same sharding, the same report.
///
/// ```no_run
/// use std::path::{Path, PathBuf};
/// use gaveldrop::adapters::{self, Adapter};
/// use gaveldrop::report::terminal::Terminal;
///
/// # fn example(mine: Box<dyn Adapter>, fake_binary: &Path) {
/// let root = Path::new(env!("CARGO_MANIFEST_DIR"));
/// let config = gaveldrop::Config::load(&root.join("gaveldrop.yaml")).unwrap();
///
/// let mut chain = vec![mine];
/// chain.extend(adapters::registry());
///
/// let mut sink = Terminal::plain(std::io::stdout());
/// let report = gaveldrop::runner::run_all_with(
///     &config, root, fake_binary, &mut sink, None, &[], &chain,
/// )
/// .unwrap();
///
/// assert!(report.is_success(), "{} case(s) failed", report.summary().failed);
/// # }
/// ```
pub fn run_all_with(
    config: &Config,
    root: &Path,
    fake_binary: &Path,
    sink: &mut dyn Sink,
    shard: Option<crate::config::Shard>,
    only: &[String],
    adapters: &[Box<dyn Adapter>],
) -> Result<Report, ConfigError> {
    let discovered = config.discover(root)?;

    // Loaded before anything runs, so two cases claiming one name stop the suite instead of producing
    // a report nobody can read. A document that will not parse is still a case failure rather than an
    // error — that distinction is the one this loop has always made, and it survives the pre-pass.
    let mut loaded: Vec<(PathBuf, Result<Case, crate::case::CaseError>)> = discovered
        .iter()
        .map(|path| (path.clone(), Case::load(path)))
        .collect();

    // **Against the whole suite, not this run's slice.** A name identifies a case in a report, and
    // reports from several shards are merged by concatenating them — so two same-named cases landing
    // on different runners would each look fine and collide only in the merged file, which is the
    // worst place to find out. Same reasoning for `--only`: running part of a suite that cannot be
    // reported on is not a smaller success.
    let named: Vec<(String, String)> = loaded
        .iter()
        .filter_map(|(path, loaded)| {
            let case = loaded.as_ref().ok()?;
            Some((case.name.clone(), path.to_string_lossy().into_owned()))
        })
        .collect();

    let collisions = crate::config::duplicate_names(&named);
    if !collisions.is_empty() {
        // Gathered only when the run is already being refused. Outside that, a case that will not
        // parse is a case failure reported alongside every other verdict, which is the more useful
        // answer: the rest of the suite still runs.
        let unreadable = loaded
            .iter()
            .filter_map(|(_, loaded)| loaded.as_ref().err())
            .map(ToString::to_string)
            .collect();
        return Err(ConfigError::DuplicateNames {
            collisions,
            unreadable,
        });
    }

    let taken = crate::config::select(discovered, shard, only)?;
    loaded.retain(|(path, _)| taken.contains(path));

    let mut outcomes = Vec::with_capacity(loaded.len());

    for (path, case) in loaded {
        let outcome = match case {
            Ok(case) => run_one(&case, fake_binary, config, root, adapters, sink),
            Err(error) => setup_failure(&path.to_string_lossy(), 0, error.to_string()),
        };
        sink.case_finished(&outcome);
        outcomes.push(outcome);
    }

    let report = Report::from(outcomes);
    sink.finish(&report);
    Ok(report)
}

/// Runs one case and records how long the whole of it took.
///
/// A wrapper rather than timing inside: every early return below is a failure, and a failure that
/// reported no duration would be the one place the number is missing — which is where a reader
/// looking for a slow setup hook would look first.
///
/// The clock covers isolation, hooks, invocation and evaluation, not the invocation alone. A slow
/// case is often slow in its `setup.exec`, and a number that excused the preparation would send the
/// reader hunting in the wrong place.
fn run_one(
    case: &Case,
    fake_binary: &Path,
    config: &Config,
    root: &Path,
    adapters: &[Box<dyn Adapter>],
    sink: &mut dyn Sink,
) -> Outcome {
    let started = std::time::Instant::now();
    let mut outcome = attempt(case, fake_binary, config, root, adapters, sink);
    outcome.duration_ms = started.elapsed().as_millis() as u64;
    outcome
}

/// Isolates, invokes and evaluates one case.
///
/// The adapter is chosen before anything is prepared: a case no adapter recognises should not cost
/// a temporary directory, and its diagnostic is more useful arriving first.
fn attempt(
    case: &Case,
    fake_binary: &Path,
    config: &Config,
    root: &Path,
    adapters: &[Box<dyn Adapter>],
    sink: &mut dyn Sink,
) -> Outcome {
    let adapter = match adapters::select(case, adapters) {
        Ok(adapter) => adapter,
        Err(error) => return setup_failure(&case.name, case.weight, error.to_string()),
    };

    let mut iso = match Isolation::prepare_with(
        case,
        fake_binary,
        &config.fake.bins,
        &config.clear_env,
        root,
        config.fake.no_passthrough,
    ) {
        Ok(iso) => iso.with_limit(crate::config::limit_for(config, case)),
        Err(error) => return setup_failure(&case.name, case.weight, error.to_string()),
    };

    sink.preparing(&case.name, &prepared(case, config, &iso, adapter.name()));

    if let Err(error) = hooks::run_setup(case, &iso, root) {
        return setup_failure(&case.name, case.weight, error.to_string());
    }

    iso.snapshot();

    let context = Context {
        defined: iso.defined(),
        invariants: config.invariants.clone(),
    };

    match adapter.invoke(case, &iso) {
        Ok(mut observations) => {
            observations.events = read_events(&observations.stdout, config);
            sink.observed(&case.name, &observations);
            let mut outcome = evaluate_in(case, &observations, &context);
            fold_in_expect_hook(&mut outcome, case, &iso, &observations, root);
            outcome
        }
        Err(error) => setup_failure(&case.name, case.weight, error.to_string()),
    }
}

/// What the engine decided, for `--verbose` to print.
///
/// The contents are not a guess at what might help: they are the questions that actually cost time
/// while putting a real project's cases on gaveldrop. Which adapter claimed this — because a case
/// naming `shell:` and `run:` goes somewhere you may not expect. Which tools are findable and which
/// were hidden — because a tool installed on the machine made a case pass here and fail on CI.
/// Which variables the case declared, resolved — because `$GAVELDROP_PROJEKT` sets something quietly
/// wrong. Where the isolated root is — because that is where you go to look at what the subject
/// wrote.
///
/// It deliberately does **not** dump the whole environment. Twenty lines of `XDG_*` per case would
/// bury the four that matter, and a reader who needs them has the root.
fn prepared(case: &Case, config: &Config, iso: &Isolation, adapter: &str) -> Vec<String> {
    let mut note = vec![
        format!("adapter    {adapter}"),
        format!("root       {}", iso.root().display()),
    ];

    // Minus what this case hides, or the trace would list the same tool as faked and hidden and
    // leave the reader to work out which won. `hide` wins; the symlink is never laid down.
    let faked: Vec<&str> = config
        .fake
        .bins
        .iter()
        .filter(|bin| !case.setup.hide.contains(bin))
        .map(String::as_str)
        .collect();
    if !faked.is_empty() {
        note.push(format!("faked      {}", faked.join(", ")));
    }
    if !case.setup.hide.is_empty() {
        note.push(format!(
            "hidden     {} (and everything else in the directories that held them)",
            case.setup.hide.join(", ")
        ));
    }
    if !case.setup.env.is_empty() {
        let defined = iso.defined();
        let resolved: Vec<String> = case
            .setup
            .env
            .keys()
            .map(|key| match defined.get(key) {
                Some(value) => format!("{key}={value}"),
                None => format!("{key}=(unresolved)"),
            })
            .collect();
        note.push(format!("env        {}", resolved.join(" ")));
    }
    if !config.clear_env.is_empty() {
        note.push(format!("cleared    {}", config.clear_env.join(", ")));
    }
    if !case.steps.is_empty() {
        note.push(format!("steps      {}", case.steps.len()));
    }

    note
}

/// Runs `expect.exec` and folds whatever it reported into `outcome`.
///
/// A protocol failure becomes a diff at `expect.exec` rather than a setup failure: the case did
/// run, and locating the problem where the case declared the hook is what lets the reader find
/// it.
fn fold_in_expect_hook(
    outcome: &mut Outcome,
    case: &Case,
    iso: &Isolation,
    observations: &Observations,
    root: &Path,
) {
    match hooks::run_expect(case, iso, observations, root) {
        Ok(diffs) => outcome.diffs.extend(diffs),
        Err(error) => outcome.diffs.push(Diff {
            path: "expect.exec".to_string(),
            expected: "a working hook".to_string(),
            got: error.to_string(),
        }),
    }

    outcome.passed = outcome.diffs.is_empty() && outcome.unexpected_calls.is_empty();
}

/// The structured events the project's configuration says to look for.
///
/// Done here rather than in the adapter: an adapter invokes and observes, and it has no
/// business knowing how this project spells its event types. A project that declares no
/// `events:` block gets none, which costs it nothing.
fn read_events(stdout: &str, config: &Config) -> Vec<Event> {
    config
        .events
        .as_ref()
        .map(|events| events::extract(stdout, events))
        .unwrap_or_default()
}

/// An outcome for a failure that happened before any expectation could be checked.
///
/// Reported the same way as a mismatch so callers never need to special-case it — the
/// difference is in the diff's path, which says `setup` rather than `expect.…`.
///
/// A case that would not parse has no trustworthy `name:`, so callers pass its path
/// instead: a report that says "the case called nothing failed" helps nobody find the file.
fn setup_failure(name: &str, weight: u32, reason: String) -> Outcome {
    Outcome {
        name: name.to_string(),
        weight,
        allow_fail: false,
        passed: false,
        diffs: vec![Diff {
            path: "setup".to_string(),
            expected: "the case runs".to_string(),
            got: reason,
        }],
        unexpected_calls: Vec::new(),
        unmentioned_files: Vec::new(),
        duration_ms: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Collects what a run reports, in the order it reports it.
    struct Recorder {
        cases: Vec<String>,
        finished: bool,
    }

    impl Sink for Recorder {
        fn case_finished(&mut self, outcome: &Outcome) {
            assert!(
                !self.finished,
                "finish() must come after every case, or a live renderer would draw a \
                 summary and then keep going"
            );
            self.cases.push(outcome.name.clone());
        }

        fn finish(&mut self, _report: &Report) {
            self.finished = true;
        }
    }

    fn project(cases: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("tests/cases")).unwrap();
        for (name, body) in cases {
            std::fs::write(dir.path().join("tests/cases").join(name), body).unwrap();
        }

        let fake = dir.path().join("gaveldrop-fake");
        std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        dir
    }

    fn config() -> Config {
        Config {
            cases: "tests/cases/**/*.yaml".to_string(),
            ..Default::default()
        }
    }

    /// A whole run of a case whose subject never returns, through the public entry point.
    ///
    /// **This is the test the finding asked for.** Before the guard, this function did not return: the
    /// case hung, the suite hung, and a continuous-integration job behind it burned until whatever
    /// global limit the runner had — often hours, since `cargo test` has no per-test timeout either. A
    /// test that hangs on regression is a poor test, but the alternative is no test at all for the one
    /// failure mode that costs money.
    #[test]
    fn a_subject_that_never_returns_fails_instead_of_hanging() {
        let dir = project(&[(
            "hangs.yaml",
            "name: hangs\nweight: 1\ntimeout: 1\nsetup:\n  run: [\"sh\", \"-c\", \"echo waiting for the provider; sleep 300\"]\nexpect:\n  exit_code: 0\n",
        )]);

        let started = std::time::Instant::now();
        let (report, _) = drive(&dir);

        assert!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "it must not have waited for the sleep: {:?}",
            started.elapsed()
        );

        let outcome = &report.outcomes[0];
        assert!(!outcome.passed);
        assert_eq!(
            outcome.diffs[0].path, "timeout",
            "and the timeout leads, not the exit code it caused: {:?}",
            outcome.diffs
        );
    }

    /// A hook that waits for something that never comes hangs the suite just as thoroughly.
    ///
    /// It was the one process in a case with no guard at all — the adapters went through a shared
    /// function and the hooks had their own copy. They share it now, which is why this passes.
    #[test]
    fn a_setup_hook_that_never_returns_fails_instead_of_hanging() {
        let dir = project(&[(
            "hooked.yaml",
            "name: hooked\nweight: 1\ntimeout: 1\nsetup:\n  run: [\"true\"]\n  exec: ./hook.sh\nexpect:\n  exit_code: 0\n",
        )]);
        let hook = dir.path().join("hook.sh");
        std::fs::write(&hook, "#!/bin/sh\nsleep 300\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

        let started = std::time::Instant::now();
        let (report, _) = drive(&dir);

        assert!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "the hook has to be killed too: {:?}",
            started.elapsed()
        );

        let outcome = &report.outcomes[0];
        assert!(!outcome.passed);
        assert!(
            outcome.diffs[0].got.contains("still running after"),
            "and it has to say the hook was killed rather than that it refused: {:?}",
            outcome.diffs
        );
    }

    const TRIVIAL: &str =
        "name: NAME\nweight: 1\nsetup:\n  run: [\"true\"]\nexpect:\n  exit_code: 0\n";

    /// Two cases claiming one name stop the run rather than producing a report nobody can read.
    ///
    /// Both used to load and both used to play, reported identically. The name is the identifier in
    /// every report this project writes, so two of them means a JUnit file several dashboards call
    /// malformed, an HTML fold that overwrites, and a terminal line that does not say which file to
    /// open. Making the second case *fail* would not have helped: the report would still carry two
    /// entries with one name.
    #[test]
    fn two_cases_claiming_one_name_stop_the_run() {
        let dir = project(&[
            ("first.yaml", &TRIVIAL.replace("NAME", "dupe")),
            ("second.yaml", &TRIVIAL.replace("NAME", "dupe")),
        ]);

        let mut recorder = Recorder {
            cases: Vec::new(),
            finished: false,
        };
        let error = run_all(
            &config(),
            dir.path(),
            &dir.path().join("gaveldrop-fake"),
            &mut recorder,
        )
        .unwrap_err();

        let said = error.to_string();
        assert!(
            said.contains("first.yaml") && said.contains("second.yaml"),
            "both files, because the fix is to open one of them: {said}"
        );
        assert!(
            recorder.cases.is_empty(),
            "and nothing ran: a suite that cannot be reported on must not half-run first"
        );
    }

    /// A refused run still reports the other thing the pre-pass found.
    ///
    /// A case that will not parse is normally a case *failure*, reported with every other verdict. But
    /// a refused run reports no verdicts, so it used to wait for a second run — one that only happens
    /// after the collision is fixed. Two problems the tool already knows about come out together, which
    /// is the same rule as `gate()` reporting every reason and `--only` naming every bad fragment.
    #[test]
    fn a_refused_run_also_names_the_cases_that_would_not_have_loaded() {
        let dir = project(&[
            ("first.yaml", &TRIVIAL.replace("NAME", "dupe")),
            ("second.yaml", &TRIVIAL.replace("NAME", "dupe")),
            ("broken.yaml", "name: broken\nweight: [\n"),
        ]);

        let mut recorder = Recorder {
            cases: Vec::new(),
            finished: false,
        };
        let error = run_all(
            &config(),
            dir.path(),
            &dir.path().join("gaveldrop-fake"),
            &mut recorder,
        )
        .unwrap_err();

        let said = error.to_string();
        assert!(said.contains("dupe"), "the collision, still first: {said}");
        assert!(
            said.contains("broken.yaml"),
            "and the case that would not load, in the same breath: {said}"
        );
    }

    /// With nothing else wrong, the refusal says nothing else.
    #[test]
    fn a_refused_run_with_nothing_else_wrong_says_nothing_else() {
        let dir = project(&[
            ("first.yaml", &TRIVIAL.replace("NAME", "dupe")),
            ("second.yaml", &TRIVIAL.replace("NAME", "dupe")),
        ]);

        let mut recorder = Recorder {
            cases: Vec::new(),
            finished: false,
        };
        let error = run_all(
            &config(),
            dir.path(),
            &dir.path().join("gaveldrop-fake"),
            &mut recorder,
        )
        .unwrap_err();

        assert!(
            !error.to_string().contains("would not have loaded"),
            "a sentence about an empty list is noise: {error}"
        );
    }

    /// A case that will not parse, on its own, is still a case failure rather than a refusal.
    ///
    /// The distinction the pre-pass had to preserve: the rest of the suite still runs, and the broken
    /// document is reported with everything else.
    #[test]
    fn a_case_that_will_not_parse_alone_is_still_a_failure_not_a_refusal() {
        let dir = project(&[
            ("fine.yaml", &TRIVIAL.replace("NAME", "fine")),
            ("broken.yaml", "name: broken\nweight: [\n"),
        ]);

        let (report, _) = drive(&dir);

        assert_eq!(report.outcomes.len(), 2, "both are reported");
        assert!(
            report.outcomes.iter().any(|outcome| outcome.passed),
            "and the good one still ran: {:?}",
            report.outcomes
        );
    }

    /// A collision outside this run's slice still stops it.
    ///
    /// Reports from several shards are merged by concatenation, so two same-named cases landing on
    /// different runners would each look fine and collide only in the merged file. Checking the whole
    /// suite is what makes the property hold there.
    #[test]
    fn a_collision_the_filter_excluded_still_stops_the_run() {
        let dir = project(&[
            ("wanted.yaml", &TRIVIAL.replace("NAME", "wanted")),
            ("first.yaml", &TRIVIAL.replace("NAME", "dupe")),
            ("second.yaml", &TRIVIAL.replace("NAME", "dupe")),
        ]);

        let mut recorder = Recorder {
            cases: Vec::new(),
            finished: false,
        };
        let error = run_all_with(
            &config(),
            dir.path(),
            &dir.path().join("gaveldrop-fake"),
            &mut recorder,
            None,
            &["wanted".to_string()],
            &adapters::registry(),
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("dupe"),
            "running part of a suite that cannot be reported on is not a smaller success: {error}"
        );
    }

    fn drive(dir: &tempfile::TempDir) -> (Report, Recorder) {
        let mut recorder = Recorder {
            cases: Vec::new(),
            finished: false,
        };
        let report = run_all(
            &config(),
            dir.path(),
            &dir.path().join("gaveldrop-fake"),
            &mut recorder,
        )
        .unwrap();
        (report, recorder)
    }

    /// An adapter no built-in resembles, claiming a key no built-in reads.
    ///
    /// It writes a marker on standard output so a passing case proves *this* code ran, rather
    /// than proving only that something did.
    struct Echo;

    impl Adapter for Echo {
        fn claims(&self, case: &Case) -> bool {
            case.setup.extra.contains_key("echo")
        }

        fn invoke(
            &self,
            _case: &Case,
            _iso: &Isolation,
        ) -> Result<Observations, adapters::AdapterError> {
            Ok(Observations {
                stdout: "ECHO-ADAPTER-RAN".to_string(),
                ..Observations::default()
            })
        }
    }

    /// An adapter that takes a known-at-least duration, so timing can be asserted at all.
    ///
    /// Sleeping rather than working: a lower bound is the only thing a clock assertion can hold
    /// without flaking, and `sleep` is the one operation that cannot finish early. Sixty
    /// milliseconds is enough to be unambiguous and short enough that the suite does not notice.
    struct Slow;

    impl Adapter for Slow {
        fn claims(&self, case: &Case) -> bool {
            case.setup.extra.contains_key("slow")
        }

        fn invoke(
            &self,
            _case: &Case,
            _iso: &Isolation,
        ) -> Result<Observations, adapters::AdapterError> {
            std::thread::sleep(std::time::Duration::from_millis(60));
            Ok(Observations::default())
        }
    }

    const SLEPT: &str = "name: slept\nweight: 1\nsetup:\n  slow: true\nexpect:\n  exit_code: 0\n";

    /// The duration reaches the outcome and the summary.
    ///
    /// Asserted as a floor and never as a ceiling. A machine under load makes any upper bound a
    /// coin toss, and a test that fails one run in two is worse than no test — which is the same
    /// reason no `expect:` key will ever gate on this number.
    #[test]
    fn a_case_reports_how_long_it_took() {
        let dir = project(&[("slept.yaml", SLEPT)]);
        let report = drive_with(&dir, &[Box::new(Slow)]);

        let outcome = &report.outcomes[0];
        assert!(outcome.passed, "the case itself has to pass: {outcome:?}");
        assert!(
            outcome.duration_ms >= 50,
            "a case that slept 60ms cannot report {}ms",
            outcome.duration_ms
        );
        assert!(
            report.summary().duration_ms >= 50,
            "and the summary has to see it, or every renderer reads zero"
        );
    }

    /// A case that never ran is timed too.
    ///
    /// The clock wraps the whole attempt rather than the invocation, so a case no adapter claims —
    /// or one whose `setup.exec` hangs before anything is invoked — still reports where the time
    /// went. That is the case a reader hunting a slow suite looks at first.
    #[test]
    fn a_setup_failure_is_timed_like_anything_else() {
        let dir = project(&[(
            "unclaimed.yaml",
            "name: unclaimed\nsetup:\n  nothing: true\n",
        )]);
        let report = drive_with(&dir, &[]);

        let outcome = &report.outcomes[0];
        assert!(!outcome.passed, "no adapter claims it: {outcome:?}");
        assert_eq!(
            report.summary().duration_ms,
            outcome.duration_ms,
            "whatever it measured, the summary counts it rather than dropping it"
        );
    }

    fn drive_with(dir: &tempfile::TempDir, adapters: &[Box<dyn Adapter>]) -> Report {
        let mut recorder = Recorder {
            cases: Vec::new(),
            finished: false,
        };
        run_all_with(
            &config(),
            dir.path(),
            &dir.path().join("gaveldrop-fake"),
            &mut recorder,
            None,
            &[],
            adapters,
        )
        .unwrap()
    }

    const ECHOED: &str = "name: echoed\nweight: 2\nsetup:\n  echo: true\nexpect:\n  exit_code: 0\n  stdout:\n    contains: [\"ECHO-ADAPTER-RAN\"]\n";

    #[test]
    fn an_adapter_the_caller_supplies_claims_and_invokes_the_case() {
        let dir = project(&[("echoed.yaml", ECHOED)]);
        let report = drive_with(&dir, &[Box::new(Echo)]);

        assert!(
            report.outcomes[0].passed,
            "a project whose cases carry vocabulary no built-in claims must be able to run its \
             own suite. The conformance kit has always taken an adapter; before this the runner \
             could not use the one it had just proved. Diffs: {:?}",
            report.outcomes[0].diffs
        );
    }

    #[test]
    fn the_same_case_has_nothing_to_invoke_without_that_adapter() {
        let dir = project(&[("echoed.yaml", ECHOED)]);
        let (report, _) = drive(&dir);

        assert!(
            !report.outcomes[0].passed,
            "this is what keeps the test above from being vacant: with the built-ins alone the \
             case must fail, so a pass there can only come from the injected adapter"
        );
        assert_eq!(report.outcomes[0].diffs[0].path, "setup");
        assert!(
            report.outcomes[0].diffs[0].got.contains("echo"),
            "and the diagnostic names the key nothing claimed: {}",
            report.outcomes[0].diffs[0].got
        );
    }

    #[test]
    fn an_adapter_placed_before_the_built_ins_wins_a_case_they_would_claim() {
        let dir = project(&[(
            "both.yaml",
            "name: both\nweight: 1\nsetup:\n  echo: true\n  run: [\"sh\", \"-c\", \"printf PROCESS-RAN\"]\nexpect:\n  exit_code: 0\n  stdout:\n    contains: [\"ECHO-ADAPTER-RAN\"]\n    absent: [\"PROCESS-RAN\"]\n",
        )]);

        let mut chain: Vec<Box<dyn Adapter>> = vec![Box::new(Echo)];
        chain.extend(adapters::registry());
        let report = drive_with(&dir, &chain);

        assert!(
            report.outcomes[0].passed,
            "the slice is searched in order, which is the only thing that lets a project \
             override a built-in for its own cases. Documented on `run_all_with`, so it is \
             promised rather than incidental. Diffs: {:?}",
            report.outcomes[0].diffs
        );
    }

    const PASSING: &str = "name: passing\nweight: 5\nsetup:\n  run: [\"sh\", \"-c\", \"true\"]\nexpect: { exit_code: 0 }\n";
    const FAILING: &str = "name: failing\nweight: 3\nsetup:\n  run: [\"sh\", \"-c\", \"exit 7\"]\nexpect: { exit_code: 0 }\n";

    #[test]
    fn every_discovered_case_is_run_and_reported_in_order() {
        let dir = project(&[("a-passing.yaml", PASSING), ("b-failing.yaml", FAILING)]);
        let (report, recorder) = drive(&dir);

        assert_eq!(recorder.cases, vec!["passing", "failing"]);
        assert!(recorder.finished, "finish() must be called once at the end");
        assert_eq!(report.summary().total, 2);
        assert_eq!(report.summary().score, 5);
        assert!(!report.is_success());
    }

    #[test]
    fn a_case_that_will_not_parse_becomes_a_failed_case_not_an_aborted_run() {
        let dir = project(&[
            ("a-broken.yaml", "this is not a case at all\n"),
            ("b-passing.yaml", PASSING),
        ]);
        let (report, recorder) = drive(&dir);

        assert_eq!(
            recorder.cases.len(),
            2,
            "the broken case must be reported and the run must continue: a load failure \
             on one file cannot take the other ninety-nine with it"
        );
        assert!(!report.outcomes[0].passed);
        assert_eq!(
            report.outcomes[0].diffs[0].path, "setup",
            "a failure before any expectation could be checked is located at `setup`, \
             which is what distinguishes it from a mismatch"
        );
        assert!(report.outcomes[1].passed);
    }

    #[test]
    fn a_broken_case_is_named_by_its_path_since_it_has_no_usable_name() {
        let dir = project(&[("unreadable.yaml", "name: [not a string]\n")]);
        let (report, _) = drive(&dir);

        assert!(
            report.outcomes[0].name.contains("unreadable.yaml"),
            "a case that would not parse has no trustworthy `name:`, so the report falls \
             back to the path. Got: {}",
            report.outcomes[0].name
        );
    }

    #[test]
    fn a_subject_that_cannot_start_becomes_a_failed_case_with_the_program_named() {
        let dir = project(&[(
            "missing-program.yaml",
            "name: no-such\nweight: 4\nsetup:\n  run: [\"no-such-program-anywhere\"]\nexpect: { exit_code: 0 }\n",
        )]);
        let (report, _) = drive(&dir);

        assert!(!report.outcomes[0].passed);
        assert_eq!(report.outcomes[0].weight, 4, "the weight must survive");
        assert!(
            report.outcomes[0].diffs[0]
                .got
                .contains("no-such-program-anywhere")
        );
    }

    #[test]
    fn a_tolerated_failure_keeps_the_run_successful() {
        let dir = project(&[(
            "known.yaml",
            "name: known\nweight: 3\nallow_fail: true\nsetup:\n  run: [\"sh\", \"-c\", \"exit 1\"]\nexpect: { exit_code: 0 }\n",
        )]);
        let (report, _) = drive(&dir);

        assert!(!report.outcomes[0].passed);
        assert!(
            report.is_success(),
            "a declared, tolerated failure must not fail the run"
        );
    }

    #[test]
    fn a_pattern_matching_nothing_stops_the_run_rather_than_reporting_success() {
        let dir = project(&[]);
        let mut recorder = Recorder {
            cases: Vec::new(),
            finished: false,
        };

        let error = run_all(
            &config(),
            dir.path(),
            &dir.path().join("gaveldrop-fake"),
            &mut recorder,
        )
        .unwrap_err();

        assert!(error.to_string().contains("tests/cases"));
        assert!(
            !recorder.finished,
            "a load error is loud and stops everything; it must not produce an empty \
             green report"
        );
    }
}
