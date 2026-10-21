//! Runs OpenSSH helper processes (`ssh -G`, `sftp -b -`) off the UI thread,
//! without a console window, with an optional timeout and cancel flag.

use std::fmt;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug)]
pub enum RunError {
    /// Could not start or wait for the process.
    Failed(String),
    /// Killed after the timeout.
    TimedOut(String),
    /// Killed because the cancel flag was set.
    Cancelled,
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Failed(m) | RunError::TimedOut(m) => f.write_str(m),
            RunError::Cancelled => f.write_str("cancelled"),
        }
    }
}

/// Number of prompts currently waiting for the user; see [`pause_timeouts`].
static PAUSES: AtomicUsize = AtomicUsize::new(0);

/// Stops timeouts from running while the guard lives, so time spent typing a
/// password for a process does not count against it.
pub fn pause_timeouts() -> TimeoutPause {
    PAUSES.fetch_add(1, Ordering::SeqCst);
    TimeoutPause(())
}

pub struct TimeoutPause(());

impl Drop for TimeoutPause {
    fn drop(&mut self) {
        PAUSES.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Runs `cmd` with `input` on stdin, killing it after `timeout`.
pub fn run(cmd: Command, input: &[u8], timeout: Duration) -> Result<Output, RunError> {
    run_with(cmd, input, Some(timeout), None)
}

/// Like [`run`], but the timeout is optional and setting `cancel` kills the
/// process. Only `cmd` itself is killed; a child it started (sftp's ssh) sees
/// its pipes close and exits on its own.
pub fn run_with(
    mut cmd: Command,
    input: &[u8],
    timeout: Option<Duration>,
    cancel: Option<&AtomicBool>,
) -> Result<Output, RunError> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| RunError::Failed(format!("cannot start {program}: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        let input = input.to_vec();
        // Dropping stdin afterwards signals end of input. A write error means
        // the process exited early, which its status reports.
        thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    // Drain both pipes on threads so a full pipe cannot stall the child.
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let stdout = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let stderr = drain(child.stderr.take().map(|p| Box::new(p) as _));

    // Running time, excluding time paused for prompts.
    let mut elapsed = Duration::ZERO;
    let mut last = Instant::now();
    let status = loop {
        let now = Instant::now();
        if PAUSES.load(Ordering::SeqCst) == 0 {
            elapsed += now - last;
        }
        last = now;
        let stop = if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            Some(RunError::Cancelled)
        } else {
            timeout.filter(|t| elapsed >= *t).map(|t| {
                RunError::TimedOut(format!(
                    "{program} did not finish within {} s",
                    t.as_secs_f32()
                ))
            })
        };
        if let Some(err) = stop {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(RunError::Failed(format!("cannot wait for {program}: {e}"))),
        }
    };
    Ok(Output {
        code: status.code(),
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn kills_a_command_that_exceeds_the_timeout() {
        let mut cmd = Command::new("ping");
        cmd.args(["-n", "30", "127.0.0.1"]);
        let started = Instant::now();
        let err = run(cmd, b"", Duration::from_millis(300))
            .err()
            .expect("times out");
        assert!(matches!(err, RunError::TimedOut(_)), "{err}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn kills_a_command_when_cancelled() {
        let mut cmd = Command::new("ping");
        cmd.args(["-n", "30", "127.0.0.1"]);
        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        let err = thread::scope(|s| {
            s.spawn(|| {
                thread::sleep(Duration::from_millis(300));
                cancel.store(true, Ordering::Relaxed);
            });
            run_with(cmd, b"", None, Some(&cancel)).err()
        })
        .expect("cancelled");
        assert!(matches!(err, RunError::Cancelled), "{err}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn paused_timeouts_do_not_expire() {
        let mut cmd = Command::new("ping");
        cmd.args(["-n", "2", "127.0.0.1"]);
        let pause = pause_timeouts();
        let out = run(cmd, b"", Duration::from_millis(300));
        drop(pause);
        assert_eq!(out.ok().and_then(|o| o.code), Some(0));
    }

    #[test]
    fn passes_stdin_and_captures_output_and_code() {
        let mut cmd = Command::new("findstr");
        cmd.arg("b");
        let out = run(cmd, "a\r\nb Ã¼\r\n".as_bytes(), Duration::from_secs(10)).unwrap();
        assert_eq!(out.code, Some(0));
        assert_eq!(out.stdout.trim(), "b Ã¼");
    }
}
