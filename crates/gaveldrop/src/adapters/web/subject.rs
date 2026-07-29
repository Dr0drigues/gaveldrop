//! A subject that has to be kept alive, and then killed.
//!
//! Every other technology in this project runs to completion. A service answers requests until
//! something stops it, and that single difference is why this type exists: starting it, draining it,
//! deciding it is ready, and above all making sure it does not outlive its case.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::Isolation;

/// A running service, killed when this value is dropped.
pub struct Subject {
    child: Child,
    port: u16,
    stdout: Arc<Mutex<String>>,
    stderr: Arc<Mutex<String>>,
    draining: Vec<std::thread::JoinHandle<()>>,
}

/// What can go wrong with a living subject.
#[derive(Debug, thiserror::Error)]
pub enum SubjectError {
    /// The service could not be started at all.
    #[error("starting the service `{program}`: {source}")]
    Spawn {
        /// The program that would not start.
        program: String,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The case named no command to serve.
    #[error("`serve:` is empty, so there is no service to start")]
    NothingToServe,
    /// The service never became ready.
    ///
    /// Carries the subject's standard error, because the reason a service did not start is almost
    /// always in there. A timeout that only says it timed out sends the reader hunting.
    #[error(
        "the service was not ready after {waited:?} (probing {probe}). Its standard error said: {}",
        if stderr.trim().is_empty() { "nothing" } else { stderr.trim() }
    )]
    NotReady {
        /// How long we waited.
        waited: Duration,
        /// What we were probing.
        probe: String,
        /// What the service wrote on standard error while we waited.
        stderr: String,
    },
}

impl Subject {
    /// Starts `argv` inside `iso`, draining both its streams from the moment it exists.
    pub fn spawn(argv: &[String], iso: &Isolation) -> Result<Self, SubjectError> {
        let (program, arguments) = argv.split_first().ok_or(SubjectError::NothingToServe)?;

        let mut command = Command::new(program);
        command
            .args(arguments)
            .current_dir(iso.root())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in iso.env() {
            command.env(key, value);
        }
        for key in iso.cleared() {
            command.env_remove(key);
        }

        let mut child = command.spawn().map_err(|source| SubjectError::Spawn {
            program: program.clone(),
            source,
        })?;

        let (stdout, reading_out) = drain(child.stdout.take());
        let (stderr, reading_err) = drain(child.stderr.take());

        Ok(Self {
            child,
            port: iso
                .defined()
                .get("GAVELDROP_PORT")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            stdout,
            stderr,
            draining: reading_out.into_iter().chain(reading_err).collect(),
        })
    }

    /// The service's process id.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// What the service has written so far.
    pub fn output(&self) -> (String, String) {
        (read(&self.stdout), read(&self.stderr))
    }

    /// Waits for the subject to finish and reports its exit code.
    ///
    /// For a case with no exchanges to perform there is nothing to be ready *for*: the subject is a
    /// process whose result is the observation. Waiting for readiness there would spend the whole
    /// timeout proving something the case never asked about.
    ///
    /// The draining threads are **joined** after the wait. A process being dead does not mean its
    /// reader has finished copying what was in the pipe, and reading the output at that moment
    /// returns whatever happened to have been transferred — intermittently nothing at all. The pipe
    /// closes when the child dies, so each reader ends on its own; this only waits for it.
    pub fn wait_for_exit(&mut self) -> i32 {
        let code = self
            .child
            .wait()
            .ok()
            .and_then(|status| status.code())
            .unwrap_or(-1);

        for reader in self.draining.drain(..) {
            let _ = reader.join();
        }

        code
    }

    /// Waits until the service answers, or gives up with a diagnostic.
    ///
    /// **Any** answer counts, including a 404: a service that replies is a service that is
    /// listening. Demanding a 2xx would make every project add a health endpoint to become
    /// testable, which is the kind of demand this project exists to avoid.
    ///
    /// With no probe URL, a TCP connection to the reserved port is used instead. It is weaker — a
    /// service can accept connections before it can serve them — so a probe is worth naming.
    pub fn wait_until_ready(
        &self,
        probe: Option<&str>,
        timeout: Duration,
    ) -> Result<(), SubjectError> {
        let deadline = Instant::now() + timeout;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(500)))
            .build()
            .into();

        while Instant::now() < deadline {
            if answered(&agent, probe, self.port) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        Err(SubjectError::NotReady {
            waited: timeout,
            probe: probe.unwrap_or("a TCP connection").to_string(),
            stderr: read(&self.stderr),
        })
    }
}

impl Drop for Subject {
    /// Kills the service and reaps it.
    ///
    /// The `wait` is not optional: without it the child becomes a zombie until gaveldrop exits, and
    /// a suite of a hundred cases would leave a hundred entries in the process table.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Whether the service answered.
///
/// With a probe URL, any reply counts — a 404 from something listening is still something listening.
/// Without one, a TCP connection to the reserved port is the fallback. It is weaker, since a service
/// can accept connections before it can serve them, which is why naming a probe is worth the line.
fn answered(agent: &ureq::Agent, probe: Option<&str>, port: u16) -> bool {
    match probe {
        Some(url) => !matches!(agent.get(url).call(), Err(ureq::Error::Io(_))),
        None => std::net::TcpStream::connect(("127.0.0.1", port)).is_ok(),
    }
}

/// Reads a stream into a string as the subject writes it.
///
/// A thread rather than a read at the end, because a pipe holds a limited amount: a subject that
/// logs more than that would block forever waiting for someone to read, and the suite would hang
/// rather than fail.
fn drain<R: std::io::Read + Send + 'static>(
    stream: Option<R>,
) -> (Arc<Mutex<String>>, Option<std::thread::JoinHandle<()>>) {
    let collected = Arc::new(Mutex::new(String::new()));
    let Some(stream) = stream else {
        return (collected, None);
    };

    let sink = Arc::clone(&collected);
    let reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            if let Ok(mut held) = sink.lock() {
                held.push_str(&line);
            }
            line.clear();
        }
    });

    (collected, Some(reader))
}

/// What has been drained so far, or an empty string if a writer panicked holding the lock.
fn read(held: &Arc<Mutex<String>>) -> String {
    held.lock().map(|text| text.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use crate::Case;

    fn case() -> Case {
        Case::load_str(
            "name: t\nweight: 1\nsetup:\n  serve: [\"true\"]\nexpect: { exit_code: 0 }\n",
            Path::new("inline"),
        )
        .unwrap()
    }

    fn isolate(outside: &Path) -> Isolation {
        let fake = outside.join("gaveldrop-fake");
        std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        Isolation::prepare(&case(), &fake, &[], &[], outside).unwrap()
    }

    fn serving(script: &str) -> Vec<String> {
        vec!["sh".to_string(), "-c".to_string(), script.to_string()]
    }

    /// A listener already bound, answering every request with `status` until it is dropped.
    ///
    /// It picks its **own** port rather than the one the isolation reserved. Reserving releases the
    /// port before returning it, so under a parallel test run another isolation can take it first —
    /// `AddrInUse`, which is the accepted gap recorded in `ROADMAP.md` showing up in our own tests.
    /// Binding `:0` and keeping the listener leaves no window at all, and what these tests check is
    /// readiness detection, not which port it happened on.
    ///
    /// Bound **synchronously** rather than inside the thread: binding in a thread races with the
    /// probe, and a bind that loses that race fails silently, leaving the test to time out for a
    /// reason that has nothing to do with what it checks. It also answers in a loop — a helper that
    /// replied once made readiness depend on the probe connecting exactly once, and the first
    /// connection to a hand-rolled listener can fail transiently.
    fn answering(status: &'static str) -> (std::net::TcpListener, u16) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("the kernel must have a port to give");
        let port = listener
            .local_addr()
            .expect("a bound listener has an address")
            .port();
        let serving = listener
            .try_clone()
            .expect("the listener must be shareable");

        std::thread::spawn(move || {
            for mut stream in serving.incoming().flatten() {
                let _ = stream.write_all(
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n").as_bytes(),
                );
            }
        });

        (listener, port)
    }

    /// Waits for a condition instead of sleeping a fixed delay.
    ///
    /// A fixed sleep is wrong by construction: too short and the test fails under load, too long
    /// and every run pays for it. These tests share a machine with a subject that writes two
    /// hundred thousand lines, so "long enough on my laptop" is not a duration that exists.
    fn until(what: &str, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if ready() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("{what} did not happen within 10s");
    }

    fn alive(pid: u32) -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[test]
    fn a_subject_that_answers_is_detected_as_ready() {
        let outside = tempfile::tempdir().unwrap();
        let iso = isolate(outside.path());
        let (_listener, port) = answering("200 OK");

        let subject = Subject::spawn(&serving("sleep 5"), &iso).unwrap();
        let url = format!("http://127.0.0.1:{port}/health");

        subject
            .wait_until_ready(Some(&url), Duration::from_secs(10))
            .expect("a service that answers must be detected as ready");
    }

    #[test]
    fn any_status_counts_as_ready_because_answering_proves_listening() {
        let outside = tempfile::tempdir().unwrap();
        let iso = isolate(outside.path());
        let (_listener, port) = answering("404 Not Found");

        let subject = Subject::spawn(&serving("sleep 5"), &iso).unwrap();
        let url = format!("http://127.0.0.1:{port}/health");

        subject
            .wait_until_ready(Some(&url), Duration::from_secs(10))
            .expect(
                "a 404 from a service that is listening still proves it is listening. Demanding a \
                 2xx would make every project add a health endpoint to become testable, which is \
                 the kind of demand this project refuses to make",
            );
    }

    #[test]
    fn a_subject_that_never_answers_times_out_rather_than_hanging() {
        let outside = tempfile::tempdir().unwrap();
        let iso = isolate(outside.path());
        let subject = Subject::spawn(&serving("sleep 30"), &iso).unwrap();

        let error = subject
            .wait_until_ready(Some("http://127.0.0.1:1/never"), Duration::from_millis(400))
            .unwrap_err();

        assert!(
            error.to_string().contains("ready"),
            "a subject that never becomes ready must fail its case with a diagnostic, never hang \
             the suite: {error}"
        );
    }

    #[test]
    fn a_timeout_carries_what_the_subject_wrote_on_stderr() {
        let outside = tempfile::tempdir().unwrap();
        let iso = isolate(outside.path());
        let subject =
            Subject::spawn(&serving("echo 'bind: address in use' >&2; sleep 30"), &iso).unwrap();
        until("the subject to write its complaint", || {
            subject.output().1.contains("address in use")
        });

        let error = subject
            .wait_until_ready(Some("http://127.0.0.1:1/never"), Duration::from_millis(400))
            .unwrap_err();

        assert!(
            error.to_string().contains("address in use"),
            "the reason a service did not start is almost always on its stderr. A timeout that \
             only says `timed out` sends the reader hunting for it: {error}"
        );
    }

    #[test]
    fn the_subject_is_dead_once_dropped() {
        let outside = tempfile::tempdir().unwrap();
        let iso = isolate(outside.path());
        let pid = {
            let subject = Subject::spawn(&serving("sleep 30"), &iso).unwrap();
            until("the subject to start", || alive(subject.pid()));
            subject.pid()
        };

        until(
            "the subject to die with the value that owned it — a subject outliving its case is a \
             process leaked onto the machine of whoever ran the tests, and a suite of a hundred \
             cases would leak a hundred",
            || !alive(pid),
        );
    }

    #[test]
    fn output_written_before_the_stop_is_still_observed() {
        let outside = tempfile::tempdir().unwrap();
        let iso = isolate(outside.path());
        let subject =
            Subject::spawn(&serving("echo listening; echo oops >&2; sleep 30"), &iso).unwrap();

        until("standard output to be drained", || {
            subject.output().0.contains("listening")
        });
        until(
            "standard error to be drained — the streams are read while the subject runs, so \
             stopping it with SIGKILL loses only what it would have written during shutdown",
            || subject.output().1.contains("oops"),
        );
    }

    #[test]
    fn a_chatty_subject_does_not_deadlock() {
        let outside = tempfile::tempdir().unwrap();
        let iso = isolate(outside.path());
        let subject = Subject::spawn(&serving("seq 1 200000; sleep 5"), &iso).unwrap();

        until(
            "a subject writing far more than a pipe buffer holds to get all of it out. It must \
             not block waiting for us to read, which is why the streams are drained by threads \
             rather than at the end",
            || subject.output().0.lines().count() > 199_000,
        );
    }

    #[test]
    fn a_service_that_cannot_start_is_an_error_not_a_panic() {
        let outside = tempfile::tempdir().unwrap();
        let iso = isolate(outside.path());

        let message = match Subject::spawn(&["no-such-service-anywhere".to_string()], &iso) {
            Ok(_) => panic!("a program that does not exist must not appear to have started"),
            Err(error) => error.to_string(),
        };
        assert!(
            message.contains("no-such-service-anywhere"),
            "a service that cannot start must fail its case with the program named, never a \
             panic that takes the other ninety-nine cases with it: {message}"
        );
    }
}
