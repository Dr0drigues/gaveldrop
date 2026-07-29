//! Applying a response: the four modes.
//!
//! | Mode | Trigger | What it does |
//! |---|---|---|
//! | static | neither `exec` nor `render` | writes `stdout`/`stderr`, exits with `exit` |
//! | render | `render` at scenario level | hands the shaping to an executable |
//! | passthrough | `exec: real` | finds the real binary further along `PATH` and calls it |
//! | delegate | `exec: <path>` | calls that executable with the same arguments |
//!
//! `exec` wins over `render`: delegating means letting the other program do the
//! writing, so there is nothing left to shape.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

use crate::{Invocation, Response, Rule};

/// What a render executable receives, as JSON, on its standard input.
///
/// **This is a public contract.** The key names are what a project script reads;
/// changing them breaks every existing script.
#[derive(Debug, Serialize)]
pub struct RenderPayload<'a> {
    /// The selected rule, with its response flattened in.
    pub rule: &'a Rule,
    /// The call as the fake saw it.
    pub invocation: &'a Invocation,
    /// The rank of this call.
    pub call: u32,
}

/// What can go wrong while applying a response.
#[derive(Debug, thiserror::Error)]
pub enum RespondError {
    /// `exec: real` was asked for but no real binary exists further along `PATH`.
    #[error("the real `{0}` is nowhere on PATH — `exec: real` requires it to be installed")]
    RealNotFound(String),
    /// A child process could not be started.
    #[error("running `{path}`: {source}")]
    Spawn {
        /// The executable that could not be run.
        path: String,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The render hook itself failed, as opposed to the faked tool failing.
    #[error("the render hook exited with {0}: that is a harness failure, not a simulated one")]
    RenderHookFailed(i32),
    /// Writing to the fake's own outputs failed.
    #[error("writing to the outputs: {0}")]
    Io(#[from] std::io::Error),
    /// The render payload could not be serialised.
    #[error("serialising the render payload: {0}")]
    Encode(#[from] serde_json::Error),
}

/// Applies `response` and returns the exit code the fake must take.
pub fn apply(
    response: &Response,
    inv: &Invocation,
    render: Option<&str>,
    payload: &RenderPayload<'_>,
) -> Result<i32, RespondError> {
    if let Some(milliseconds) = response.latency_ms {
        std::thread::sleep(std::time::Duration::from_millis(milliseconds));
    }

    match response.exec.as_deref() {
        Some("real") => passthrough(inv),
        Some(path) => delegate(path, inv),
        None => match render {
            Some(script) => {
                let status = render_with(script, payload)?;
                exit_after_render(response.exit, status)
            }
            None => write_static(response),
        },
    }
}

/// Which exit code the fake takes once a render hook has run.
///
/// The hook shapes bytes; it does **not** decide whether the faked tool succeeded. So
/// the rule's `exit` stands, and a non-zero exit from the hook means the hook itself
/// broke — a harness failure, which must never be passed off as the faked tool's exit
/// code. Otherwise a scenario could not say "this tool failed" and shape its output at
/// the same time.
fn exit_after_render(rule_exit: Option<i32>, hook_status: i32) -> Result<i32, RespondError> {
    if hook_status != 0 {
        return Err(RespondError::RenderHookFailed(hook_status));
    }
    Ok(rule_exit.unwrap_or(0))
}

/// Static mode: write what the rule says, exit with its code.
fn write_static(response: &Response) -> Result<i32, RespondError> {
    if let Some(out) = &response.stdout {
        write_line(&mut std::io::stdout().lock(), out)?;
    }
    if let Some(err) = &response.stderr {
        write_line(&mut std::io::stderr().lock(), err)?;
    }
    Ok(response.exit.unwrap_or(0))
}

/// Writes `text`, adding the trailing newline a real command-line tool would emit.
fn write_line(sink: &mut impl Write, text: &str) -> std::io::Result<()> {
    sink.write_all(text.as_bytes())?;
    if !text.ends_with('\n') {
        sink.write_all(b"\n")?;
    }
    sink.flush()
}

/// Render mode: the script receives the payload as JSON and writes the bytes.
///
/// Its standard output and error are inherited: what it writes goes straight to the
/// caller without passing through us. Copying would be work and an opportunity to
/// truncate.
///
/// The script's standard input is closed explicitly after writing: without that, a
/// script reading to end-of-input would wait forever.
fn render_with(script: &str, payload: &RenderPayload<'_>) -> Result<i32, RespondError> {
    let json = serde_json::to_vec(payload)?;

    let mut child = Command::new(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|source| RespondError::Spawn {
            path: script.to_string(),
            source,
        })?;

    if let Some(mut input) = child.stdin.take() {
        input.write_all(&json)?;
        drop(input);
    }

    Ok(child.wait()?.code().unwrap_or(1))
}

/// Runs the render hook and **returns** what it wrote, rather than letting it write for us.
///
/// The binary door lets the hook inherit its own streams: the fake *is* the process the subject
/// invoked, so bytes on stdout are already in the right place. A service has no such luck — the bytes
/// have to become a response body, which means capturing them.
///
/// One hook protocol, two consumers. A project writes the same `fake.render` executable and it works
/// at either door, which is the same guarantee the rules already carry.
pub fn rendered_bytes(
    script: &str,
    payload: &RenderPayload<'_>,
) -> Result<(String, i32), RespondError> {
    let json = serde_json::to_vec(payload)?;

    let mut child = Command::new(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|source| RespondError::Spawn {
            path: script.to_string(),
            source,
        })?;

    if let Some(mut input) = child.stdin.take() {
        input.write_all(&json)?;
        drop(input);
    }

    let output = child.wait_with_output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code().unwrap_or(1),
    ))
}

/// Passthrough mode: call the real binary, found further along `PATH`.
fn passthrough(inv: &Invocation) -> Result<i32, RespondError> {
    let self_exe = std::env::current_exe().ok();
    let path = std::env::var("PATH").unwrap_or_default();

    let real = real_binary_in(&inv.bin, &path, self_exe.as_deref())
        .ok_or_else(|| RespondError::RealNotFound(inv.bin.clone()))?;

    run_inheriting(&real, inv)
}

/// Delegate mode: call the project executable with the same arguments.
fn delegate(path: &str, inv: &Invocation) -> Result<i32, RespondError> {
    run_inheriting(Path::new(path), inv)
}

/// Runs `program` with the call's arguments, every stream inherited.
fn run_inheriting(program: &Path, inv: &Invocation) -> Result<i32, RespondError> {
    let status = Command::new(program)
        .args(&inv.args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|source| RespondError::Spawn {
            path: program.display().to_string(),
            source,
        })?;
    Ok(status.code().unwrap_or(1))
}

/// Looks `bin` up in `path`, skipping any candidate that **is** `own_executable`.
///
/// Skipping ourselves is what stops passthrough from calling itself forever: the fake
/// sits first on `PATH` under precisely the name of the binary it stands in for.
///
/// The skip is by **file identity, not by directory**, and that distinction is
/// load-bearing. `std::env::current_exe` resolves symlinks on Linux (`/proc/self/exe`)
/// but not on macOS (`_NSGetExecutablePath` returns the path as invoked), so comparing
/// directories works on one platform and silently fails on the other — where the fake
/// then finds itself and recurses until `fork` gives out. Canonicalising both sides
/// removes the platform difference, and skips our binary wherever on `PATH` it appears
/// rather than only in one blessed directory.
pub fn real_binary_in(bin: &str, path: &str, own_executable: Option<&Path>) -> Option<PathBuf> {
    let own = own_executable.and_then(|path| std::fs::canonicalize(path).ok());

    path.split(':')
        .filter(|entry| !entry.is_empty())
        .map(|dir| Path::new(dir).join(bin))
        .filter(|candidate| is_executable(candidate))
        .find(|candidate| !is_same_file(candidate, own.as_deref()))
}

/// True when `candidate` resolves to the same file as `own`, symlinks followed.
fn is_same_file(candidate: &Path, own: Option<&Path>) -> bool {
    let Some(own) = own else { return false };
    std::fs::canonicalize(candidate).is_ok_and(|resolved| resolved == own)
}

/// True when the path is a file carrying at least one execute bit.
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Match;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn executable(path: &std::path::Path) {
        fs::write(path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn real_binary_skips_our_own_executable_even_through_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let ours = dir.path().join("the-fake-itself");
        executable(&ours);

        let first = dir.path().join("a");
        let second = dir.path().join("b");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        std::os::unix::fs::symlink(&ours, first.join("git")).unwrap();
        executable(&second.join("git"));

        let path = format!("{}:{}", first.display(), second.display());
        assert_eq!(
            real_binary_in("git", &path, Some(&ours)).unwrap(),
            second.join("git"),
            "the first `git` on PATH is a symlink to us; following it would recurse \
             until fork gives out. The skip must be by file identity, not by directory: \
             current_exe resolves symlinks on Linux but not on macOS, so a \
             directory comparison passes on one platform and silently fails on the other"
        );
    }

    #[test]
    fn real_binary_skips_our_own_executable_named_directly() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("a");
        let second = dir.path().join("b");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();

        let ours = first.join("git");
        executable(&ours);
        executable(&second.join("git"));

        let path = format!("{}:{}", first.display(), second.display());
        assert_eq!(
            real_binary_in("git", &path, Some(&ours)).unwrap(),
            second.join("git"),
            "the same must hold when our own executable is the PATH entry itself, not a \
             symlink to it"
        );
    }

    #[test]
    fn real_binary_ignores_non_executables() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("a");
        let second = dir.path().join("b");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();

        let not_executable = first.join("jq");
        fs::write(&not_executable, "").unwrap();
        fs::set_permissions(&not_executable, fs::Permissions::from_mode(0o644)).unwrap();
        let real = second.join("jq");
        executable(&real);

        let path = format!("{}:{}", first.display(), second.display());
        assert_eq!(real_binary_in("jq", &path, None).unwrap(), real);
    }

    #[test]
    fn real_binary_returns_none_when_the_real_binary_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            real_binary_in("does-not-exist", &dir.path().display().to_string(), None).is_none()
        );
    }

    #[test]
    fn the_render_payload_is_stable_json() {
        let inv = Invocation {
            bin: "claude".into(),
            args: vec!["-p".into(), "hello".into()],
            stdin: String::new(),
        };
        let rule = Rule {
            matcher: Match::default(),
            response: Response {
                stdout: Some("ok".into()),
                ..Default::default()
            },
        };
        let payload = RenderPayload {
            rule: &rule,
            invocation: &inv,
            call: 2,
        };
        let value: serde_json::Value = serde_json::to_value(&payload).unwrap();

        let contract = "the protocol's keys are a contract: a render script written \
                        today must keep working tomorrow";
        assert_eq!(value["call"], 2, "{contract}");
        assert_eq!(value["invocation"]["bin"], "claude", "{contract}");
        assert_eq!(value["rule"]["stdout"], "ok", "{contract}");
    }

    #[test]
    fn a_render_hook_does_not_decide_whether_the_faked_tool_failed() {
        assert_eq!(
            exit_after_render(Some(1), 0).unwrap(),
            1,
            "the hook shapes bytes; the rule still decides whether the faked tool \
             succeeded"
        );
        assert_eq!(exit_after_render(None, 0).unwrap(), 0);
    }

    #[test]
    fn a_render_hook_that_fails_is_a_harness_failure_not_a_simulated_one() {
        let error = exit_after_render(Some(0), 3).unwrap_err();
        assert!(
            error.to_string().contains("render hook"),
            "a broken hook must be reported as our failure, never passed off as the \
             faked tool's exit code: {error}"
        );
    }
}
