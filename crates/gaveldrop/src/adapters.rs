//! Adapters: invoke the subject, return normalised observations.

pub mod process;
pub mod shell;
pub mod web;

use std::io::Read;
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{Case, Isolation, Observations};

pub use process::Process;
pub use shell::Shell;
pub use web::Web;

/// Invokes the subject and returns what it produced.
///
/// An adapter invokes and observes. It **never evaluates** — no adapter knows what a case
/// expects. That is what guarantees an expectation written once behaves identically
/// whatever the technology.
pub trait Adapter {
    /// Whether this adapter is the one for `case`.
    ///
    /// Declaring a capability, not judging a result: the invariant above is about expectations, and
    /// this reads none. Selection lives in the case rather than in configuration so that a project
    /// mixing a binary and the shell scripts around it does not have to split its suite.
    fn claims(&self, case: &Case) -> bool;

    /// Runs `case` inside `iso` and reports what happened.
    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError>;

    /// How this adapter calls itself, for the trace `--verbose` prints.
    ///
    /// Defaulted rather than required, unlike `claims`. The reasoning is the opposite of the one
    /// there: a wrong default for `claims` fails quietly, whereas a nameless adapter only makes one
    /// line of a diagnostic vaguer. Requiring it would break every consumer adapter already written
    /// against this trait — which right now means armadai's, mid-migration — for a label.
    fn name(&self) -> &str {
        "a consumer-provided adapter"
    }
}

/// Every adapter, in the order they are asked.
///
/// Order matters at exactly one point: a case could plausibly name both `serve:` and `shell:`, since
/// starting a service written as a shell function is not absurd. The more specific claim has to win,
/// so `Web` is asked first.
pub fn registry() -> Vec<Box<dyn Adapter>> {
    vec![Box::new(Web), Box::new(Shell), Box::new(Process)]
}

/// Reduces a cumulative call journal to what each exchange added to it.
///
/// **The journal only ever grows, and every adapter reads all of it.** So an exchange handed the whole
/// journal saw its own calls plus every call before it: `calls: { git: 1 }` held on the first exchange
/// and failed on the second with `got 2`. That makes a per-exchange count depend on everything
/// upstream — insert an exchange and every count after it is wrong — which is the assertion
/// `docs/writing-cases.md` exists to talk people out of writing.
///
/// The file effects of an exchange were already segmented, by a snapshot taken before and after. This
/// is the same idea for a stream that only appends, where an offset is all a snapshot would amount to.
///
/// The run **as a whole** still sees every call, since that is what `expect.calls` at the top level
/// asks about. Reported by the second consumer, whose exchanges each call the same tool once.
pub(crate) struct Ledger {
    counted: usize,
}

impl Ledger {
    /// A ledger positioned before the first exchange.
    pub(crate) fn new() -> Self {
        Self { counted: 0 }
    }

    /// Cuts `calls` — the journal as it stands — down to the ones this exchange added.
    pub(crate) fn only_the_new(&mut self, calls: &mut Vec<gaveldrop_fake::Call>) {
        // `min` rather than a bare range: a journal that came back shorter than the last one is not
        // something this can produce, and panicking on it would turn a surprise into a lost suite.
        calls.drain(..self.counted.min(calls.len()));
        self.counted += calls.len();
    }
}

/// A finished invocation, and whether it finished by itself.
#[derive(Debug)]
pub struct Completed {
    /// What the subject exited with and wrote.
    pub output: Output,
    /// The limit, in milliseconds, the subject was killed for exceeding — absent when it exited itself.
    ///
    /// Milliseconds because that is the unit every duration in this project is stored in, and because
    /// seconds would report a sub-second limit as `0`. The configuration is in whole seconds; this is
    /// what the guard actually applied.
    pub timed_out_after_ms: Option<u64>,
}

/// Runs `command`, feeding it `stdin` when the case declared one, and killing it after `limit`.
///
/// Shared rather than written in each adapter, because there are two traps in here and an adapter
/// author should meet neither.
///
/// **The pipe.** Writing the input on this thread and *then* reading the output deadlocks as soon as
/// the subject fills its own output pipe before it has finished reading its input — a filter over
/// more than a pipe's worth of data does exactly that. So the input goes out on its own thread while
/// the two output pipes are drained on theirs.
///
/// A write that fails is ignored on purpose: a subject is entitled to stop reading — `head -1` does
/// — and closing the pipe early is its business, not an error in the case.
///
/// **The hang.** `limit` of `None` means no limit, and it is not the default anywhere a case reaches:
/// a subject that never returns used to hang the case, the suite and the continuous-integration job
/// with it, until whatever global timeout the runner had — often hours. Reported by the first
/// consumer with an adapter of its own, whose subject calls a network provider that can simply not
/// answer.
pub fn invoke(
    command: &mut Command,
    stdin: Option<&str>,
    limit: Option<Duration>,
) -> std::io::Result<Completed> {
    command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Its own process group, so a timeout can kill everything the subject started rather than the
        // subject alone. See `kill_group`, which is the half that needs it.
        .process_group(0);

    let mut child = command.spawn()?;

    let writer = stdin.map(str::to_string).and_then(|payload| {
        child.stdin.take().map(|mut pipe| {
            std::thread::spawn(move || {
                use std::io::Write;
                let _ = pipe.write_all(payload.as_bytes());
            })
        })
    });

    let completed = wait_within(&mut child, limit)?;

    // Not joined when the subject was killed: a grandchild can still hold the input pipe, and this
    // thread would then be waiting on a write nobody reads — turning the timeout into the very hang
    // it exists to prevent.
    if let Some(writer) = writer
        && completed.timed_out_after_ms.is_none()
    {
        let _ = writer.join();
    }

    Ok(completed)
}

/// Waits for `child`, killing it once `limit` has passed, and returns what it wrote either way.
///
/// **Whatever the killed subject wrote is kept**, on both streams, rather than thrown away as a
/// leftover. A subject that hangs has nearly always printed the thing it hung on, and a report saying
/// only that something took too long leaves the reader nowhere.
///
/// Polled rather than waited on, with a backoff. `Child::wait` blocks with no deadline and killing
/// needs the same `&mut`, so no arrangement of the standard library waits and kills at once; doing it
/// by signal would mean a `libc` dependency in a project with fourteen. The backoff costs nothing
/// measurable — timed against a plain `Command::output()` on the same command, 8.6ms against 8.9ms.
fn wait_within(child: &mut Child, limit: Option<Duration>) -> std::io::Result<Completed> {
    let mut out = child.stdout.take().map(drain);
    let mut err = child.stderr.take().map(drain);

    let mut timed_out_after_ms = None;

    let status = match limit {
        None => child.wait()?,
        Some(limit) => {
            let deadline = Instant::now() + limit;
            let mut nap = FIRST_NAP;

            loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                if Instant::now() >= deadline {
                    kill_group(child);
                    timed_out_after_ms = Some(limit.as_millis() as u64);
                    // Long enough for the readers to see the pipes close, short enough that nobody
                    // notices. Not a join: see the grandchild note above.
                    std::thread::sleep(LAST_WORDS);
                    break child.wait()?;
                }
                std::thread::sleep(nap.min(deadline.saturating_duration_since(Instant::now())));
                nap = (nap * 2).min(LONGEST_NAP);
            }
        }
    };

    // Waited for unless the subject was killed. A reader that has not finished has not necessarily
    // handed over the last chunk, and reading its buffer early loses output on a run that did nothing
    // wrong — which is how a plain `echo` came back empty under load. On the killed path the wait is
    // the one thing that must not happen: a grandchild can still hold the pipe, and the reader would
    // then block for ever on a subject nobody is going to close.
    if timed_out_after_ms.is_none() {
        for reader in [out.as_mut(), err.as_mut()].into_iter().flatten() {
            reader.finish();
        }
    }

    Ok(Completed {
        output: Output {
            status,
            stdout: out.map(Reader::collected).unwrap_or_default(),
            stderr: err.map(Reader::collected).unwrap_or_default(),
        },
        timed_out_after_ms,
    })
}

/// Kills the subject and everything it started.
///
/// **`Child::kill` sends `SIGKILL` to one process**, so a subject that had started anything of its own
/// left it running: a case timing out on `sh -c '(sleep 300 &) ; sleep 300'` left an orphan reparented
/// to `init`, and a continuous-integration job would accumulate one per timeout. Measured before and
/// after: two survivors with `PPID 1`, then none.
///
/// Killing a process group needs a negative pid, which `Child::kill` cannot express and `kill(2)`
/// would — but `unsafe_code` is forbidden in this workspace and `libc` is not a dependency, so a shell
/// does it.
///
/// **Through `sh -c`, not by running `kill` directly.** A negative first argument is where the
/// implementations disagree: `/bin/kill` from procps read `-1234` as an option rather than as a group,
/// so on Linux the orphans survived while macOS was clean. The shell **builtin** takes it as POSIX
/// specifies on both, which the continuous-integration run is what proved — this fix went out once
/// working on one platform and failing on the other.
///
/// **The cost, stated because it is real:** the subject is no longer in the terminal's foreground
/// process group, so `Ctrl-C` during a run reaches gaveldrop and not the subject. Closing that would
/// mean handling `SIGINT`, which needs one of the two things above. The trade was made this way round
/// because a timeout is automated and silent while an interrupt is interactive and visible.
pub(crate) fn kill_group(child: &mut Child) {
    let _ = Command::new("sh")
        .args(["-c", &format!("kill -9 -{}", child.id())])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // Belt and braces: if `kill` is missing or refuses, the subject itself still goes.
    let _ = child.kill();
}

/// The first pause between two checks, short enough that a fast case pays almost nothing.
const FIRST_NAP: Duration = Duration::from_micros(200);
/// The longest pause between two checks, so a subject running for minutes costs a few hundred wakeups.
const LONGEST_NAP: Duration = Duration::from_millis(50);
/// How long a killed subject's readers get to finish before its output is read.
const LAST_WORDS: Duration = Duration::from_millis(50);

/// One pipe being read on its own thread, into a buffer this thread can look at either way.
///
/// The buffer is shared rather than returned by the thread, because on the killed path waiting for
/// that thread is the one thing that must not happen: a grandchild can still hold the pipe open, and
/// the read would never return. Everywhere else the thread is waited for, since a reader that has not
/// finished has not necessarily handed over the last chunk.
struct Reader {
    buffer: Arc<Mutex<Vec<u8>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Reader {
    /// Waits for the pipe to reach end of input, so nothing written is missed.
    fn finish(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    /// Whatever the pipe has yielded so far.
    fn collected(self) -> Vec<u8> {
        self.buffer
            .lock()
            .map(|held| held.clone())
            .unwrap_or_default()
    }
}

/// Starts reading `pipe` on a thread of its own.
fn drain(mut pipe: impl Read + Send + 'static) -> Reader {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&buffer);

    let thread = std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if let Ok(mut held) = sink.lock() {
                        held.extend_from_slice(&chunk[..read]);
                    }
                }
            }
        }
    });

    Reader {
        buffer,
        thread: Some(thread),
    }
}

/// The adapter for `case`.
///
/// Each is asked in turn rather than trying `invoke` until one succeeds: trying would run the
/// subject, and a case must never execute against an adapter that was not meant for it.
pub fn select<'a>(
    case: &Case,
    from: &'a [Box<dyn Adapter>],
) -> Result<&'a dyn Adapter, AdapterError> {
    from.iter()
        .find(|adapter| adapter.claims(case))
        .map(AsRef::as_ref)
        .ok_or_else(|| AdapterError::Unclaimed {
            case: case.name.clone(),
            keys: case.setup.extra.keys().cloned().collect(),
        })
}

/// What can go wrong while invoking a subject.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    /// The case does not give this adapter enough to work with.
    #[error("case `{case}` cannot be invoked by this adapter: {reason}")]
    Unsupported {
        /// The case's name.
        case: String,
        /// What is missing.
        reason: String,
    },
    /// The subject could not be started.
    #[error("starting `{program}`: {source}")]
    Spawn {
        /// The program that would not start.
        program: String,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The call journal could not be read back.
    #[error("reading the call journal: {0}")]
    Journal(#[from] gaveldrop_fake::JournalError),
    /// No adapter recognised the case, so nothing would be invoked.
    ///
    /// This is the refusal that used to live in `Case::load`, and it guards the same thing: a case
    /// that parses and then invokes nothing is a green test asserting about a program that never
    /// started. It moved here because `run` and `exec` stopped being the criterion the moment there
    /// was more than one adapter.
    ///
    /// The listed keys are the rest of the diagnostic. Because `Setup::extra` accepts any key by
    /// design, a mistyped one cannot be rejected by the format — so the reader is shown what was
    /// actually there and spots `shel` against `shell` themselves.
    #[error(
        "case `{case}` would invoke nothing: no adapter recognises it. Add `run: [...]` with a \
         command line, `shell:` with `call:` for a shell function, or `serve:` for a service. \
         `setup.exec` only prepares \
         the directory, so it is not enough on its own. setup holds {}",
        if keys.is_empty() { "no other key".to_string() } else { keys.join(", ") }
    )]
    Unclaimed {
        /// The case's name.
        case: String,
        /// The keys `setup` did hold.
        keys: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn case(yaml: &str) -> Case {
        Case::load_str(yaml, Path::new("inline")).unwrap()
    }

    /// A limit generous enough that only a hang trips it, measured rather than guessed.
    ///
    /// **A constant here was a flake**, and the second one of the same kind. Starting a process is not
    /// a fixed cost: the first one this binary starts takes half a second on an idle machine — a plain
    /// `Command::output()` pays 444ms against the 8ms the fourth spawn takes — and under the full test
    /// gate, four hundred tests deep, it went past the one second a constant allowed. The subject was
    /// then killed while it was still starting, having written nothing, and a test asserting on its
    /// output failed for a reason that had nothing to do with the code.
    ///
    /// So the cost is measured here and now, and the limit is ten times it. A loaded machine measures
    /// slowly and gets a proportionally longer limit; an idle one pays the floor and moves on. The
    /// measurement doubles as the warm-up the first spawn needs.
    fn generous() -> Duration {
        let started = Instant::now();
        let _ = Command::new("sh").args(["-c", ":"]).status();

        // A floor, because a warm spawn measures at single-digit milliseconds and ten of those is not
        // long enough for a subject to start, write and be waited on.
        (started.elapsed() * 10).max(Duration::from_millis(500))
    }

    const WITH_RUN: &str =
        "name: t\nweight: 1\nsetup:\n  run: [\"true\"]\nexpect: { exit_code: 0 }\n";
    const WITH_SHELL: &str =
        "name: t\nweight: 1\nsetup:\n  shell: bash\n  call: [\"f\"]\nexpect: { exit_code: 0 }\n";

    /// A subject that never returns is killed, and says it was.
    ///
    /// The finding this exists for, from the first consumer with an adapter of its own: a hanging
    /// subject used to hang the case, the suite and the continuous-integration job behind it. `cargo
    /// test` has no per-test timeout either, so the job burned until whatever global limit the runner
    /// had — often hours.
    #[test]
    fn a_subject_that_outlasts_its_limit_is_killed() {
        let limit = generous();
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30"]);

        let started = Instant::now();
        let completed = invoke(&mut command, None, Some(limit)).unwrap();

        assert_eq!(
            completed.timed_out_after_ms,
            Some(limit.as_millis() as u64),
            "killed, and the limit it outlasted is reported so the verdict can name it"
        );
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "it must not have waited for the sleep: {:?}",
            started.elapsed()
        );
    }

    /// Everything the subject started dies with it.
    ///
    /// `Child::kill` sends `SIGKILL` to one process, so a subject that had started anything of its own
    /// left it running — reparented to `init`, and one more per timeout on a continuous-integration
    /// machine. Reported by the shell adapter's consumer, who looked at `ps` rather than at whether
    /// gaveldrop returned.
    ///
    /// The tag is unusual on purpose: this reads the real process table, so it has to match nothing
    /// else on the machine.
    #[test]
    fn a_timeout_takes_the_subjects_descendants_with_it() {
        let mut command = Command::new("sh");
        command.args(["-c", "(sleep 294 &) ; sleep 294"]);

        let completed = invoke(&mut command, None, Some(generous())).unwrap();
        assert!(completed.timed_out_after_ms.is_some());

        // Long enough for the group kill to have been delivered and reaped.
        std::thread::sleep(Duration::from_millis(300));

        let table = Command::new("ps")
            .args(["-eo", "pid,ppid,command"])
            .output()
            .unwrap();
        // The command column matched exactly, not searched for. A `contains` here also matched the
        // shell line that had *typed* the script, so a failure printed a screenful of unrelated
        // process and a passing machine could have failed for mentioning the word.
        let listing = String::from_utf8_lossy(&table.stdout).into_owned();
        let survivors: Vec<&str> = listing
            .lines()
            .filter(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                fields.get(2..) == Some(&["sleep", "294"])
            })
            .map(str::trim)
            .collect();

        assert!(
            survivors.is_empty(),
            "a subject killed on a timeout must not leave its own children behind. If this fails on \
             one platform and not the other, look at how that platform's `kill` reads a negative \
             first argument: {survivors:#?}"
        );
    }

    /// What a killed subject wrote is kept, on both streams, so the reader has somewhere to start.
    #[test]
    fn what_a_killed_subject_wrote_is_still_reported() {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "echo out of the fetch; echo into the wait >&2; sleep 30",
        ]);

        let completed = invoke(&mut command, None, Some(generous())).unwrap();

        assert!(completed.timed_out_after_ms.is_some());
        assert!(
            String::from_utf8_lossy(&completed.output.stdout).contains("out of the fetch"),
            "a report saying only that something took too long leaves the reader nowhere: {:?}",
            String::from_utf8_lossy(&completed.output.stdout)
        );
        assert!(
            String::from_utf8_lossy(&completed.output.stderr).contains("into the wait"),
            "and progress is usually on the other stream: {:?}",
            String::from_utf8_lossy(&completed.output.stderr)
        );
    }

    /// A subject that finishes inside its limit is untouched by the guard.
    #[test]
    fn a_subject_that_finishes_in_time_is_not_marked() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 0.2; echo done"]);

        let completed = invoke(&mut command, None, Some(Duration::from_secs(60))).unwrap();

        assert_eq!(completed.timed_out_after_ms, None);
        assert!(completed.output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&completed.output.stdout).trim(),
            "done",
            "the polling loop must not eat the output it was waiting for"
        );
    }

    /// No limit still means no limit, for the suite that legitimately runs for as long as it needs.
    #[test]
    fn no_limit_runs_to_completion() {
        let mut command = Command::new("sh");
        command.args(["-c", "echo unhurried"]);

        let completed = invoke(&mut command, None, None).unwrap();

        assert_eq!(completed.timed_out_after_ms, None);
        assert_eq!(
            String::from_utf8_lossy(&completed.output.stdout).trim(),
            "unhurried"
        );
    }

    /// Every byte a subject wrote is there, however many reads it took.
    ///
    /// The test that caught the race: the readers were not waited for on the path where nothing was
    /// killed, so their last chunk could still be in flight when the buffer was read. It showed up as a
    /// plain `echo` coming back empty under load — a run that had done nothing wrong losing its output.
    /// More than one pipe's worth, so the reader is guaranteed several reads to be raced against.
    #[test]
    fn nothing_a_subject_wrote_is_lost_when_it_was_not_killed() {
        let mut command = Command::new("sh");
        // One word, because BSD `yes` takes one argument where GNU `yes` joins them all — a
        // multi-word version of this test measured the shell rather than the reader.
        command.args(["-c", "yes payload | head -20000"]);

        let completed = invoke(&mut command, None, Some(Duration::from_secs(60))).unwrap();

        assert_eq!(completed.timed_out_after_ms, None);
        assert_eq!(
            completed.output.stdout.len(),
            20_000 * "payload\n".len(),
            "every byte, or the guard has quietly become a way to lose output"
        );
    }

    /// Standard input still reaches a subject through the guarded path.
    ///
    /// The pipe trap and the hang guard are now the same function, and the input thread is only
    /// joined when the subject was not killed. A regression here would look like a lost stdin rather
    /// than like a timeout.
    #[test]
    fn stdin_still_reaches_the_subject() {
        let mut command = Command::new("cat");

        let completed =
            invoke(&mut command, Some("fed in"), Some(Duration::from_secs(60))).unwrap();

        assert_eq!(
            String::from_utf8_lossy(&completed.output.stdout).trim(),
            "fed in"
        );
    }

    /// A subject that stops reading its input is not an error.
    ///
    /// `head -1` does exactly this, and the write that fails is ignored on purpose. Worth asserting
    /// now that the write thread is sometimes not joined: an unwrap in either place would turn a
    /// legitimate subject into a spurious failure.
    #[test]
    fn a_subject_that_stops_reading_its_input_is_not_a_failure() {
        let mut command = Command::new("head");
        command.args(["-1"]);

        let completed = invoke(
            &mut command,
            Some(&"a line\n".repeat(50_000)),
            Some(Duration::from_secs(60)),
        )
        .unwrap();

        assert_eq!(completed.timed_out_after_ms, None);
        assert_eq!(
            String::from_utf8_lossy(&completed.output.stdout).trim(),
            "a line",
            "closing the pipe early is the subject's business, not an error in the case"
        );
    }

    #[test]
    fn a_case_with_run_goes_to_an_adapter_that_takes_a_command_line() {
        let adapters = registry();
        let chosen = select(&case(WITH_RUN), &adapters).unwrap();

        assert!(chosen.claims(&case(WITH_RUN)));
        assert!(
            !chosen.claims(&case(WITH_SHELL)),
            "the two adapters must not both claim everything, or the registry order decides by \
             accident rather than the case deciding"
        );
    }

    #[test]
    fn a_case_with_shell_goes_to_the_shell_adapter() {
        let adapters = registry();
        let chosen = select(&case(WITH_SHELL), &adapters).unwrap();

        assert!(chosen.claims(&case(WITH_SHELL)));
        assert!(!chosen.claims(&case(WITH_RUN)));
    }

    fn refusal(yaml: &str) -> String {
        let adapters = registry();
        match select(&case(yaml), &adapters) {
            Ok(_) => panic!("some adapter claimed a case that names neither `run` nor `shell`"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn a_mistyped_key_is_told_which_keys_were_seen() {
        let message = refusal(
            "name: t\nweight: 1\nsetup:\n  shel: bash\n  call: [\"f\"]\nexpect: { exit_code: 0 }\n",
        );

        for expected in ["shel", "call"] {
            assert!(
                message.contains(expected),
                "`extra` accepts any key by design, so a typo cannot be rejected by the format. \
                 Listing what was seen is what lets a reader spot it without opening our source. \
                 Missing {expected:?} in: {message}"
            );
        }
    }

    #[test]
    fn a_case_claimed_by_no_one_names_the_case_and_says_what_would_work() {
        let message = refusal("name: lonely\nweight: 1\nsetup: {}\nexpect: { exit_code: 0 }\n");

        assert!(message.contains("lonely"));
        assert!(
            message.contains("run") && message.contains("shell"),
            "naming the keys that would have worked is what turns a dead end into a next step: \
             {message}"
        );
    }
}
