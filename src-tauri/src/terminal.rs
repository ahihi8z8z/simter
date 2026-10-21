//! Interactive processes hosted in a pseudoconsole (ConPTY on Windows).

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exit {
    /// Process exit code, when it could be determined.
    pub code: Option<u32>,
    /// Why the exit status is unknown, if it is.
    pub error: Option<String>,
}

/// A running process attached to a pseudoconsole.
pub struct PtySession {
    // Taken (and so closed) once the process exits, which ends the reader.
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    // Feeds a writer thread, so a full input pipe never blocks the caller.
    input: Mutex<Option<Sender<Vec<u8>>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl PtySession {
    /// Starts `cmd` in a new pseudoconsole. Output chunks are passed to
    /// `on_output` from a reader thread. `on_exit` runs once, after the
    /// process has exited and all of its output has been delivered.
    pub fn spawn(
        cmd: CommandBuilder,
        cols: u16,
        rows: u16,
        mut on_output: impl FnMut(&[u8]) + Send + 'static,
        on_exit: impl FnOnce(Exit) + Send + 'static,
    ) -> Result<Arc<PtySession>, String> {
        let pair = native_pty_system()
            .openpty(size(cols, rows))
            .map_err(|e| format!("cannot create pseudoconsole: {e}"))?;
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("cannot start process: {e}"))?;
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("cannot read pseudoconsole: {e}"))?;
        let mut writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("cannot write pseudoconsole: {e}"))?;
        let (input, pending) = mpsc::channel::<Vec<u8>>();
        // Ends when `input` is dropped or the pipe breaks; later writes then fail.
        thread::spawn(move || {
            for data in pending {
                if writer
                    .write_all(&data)
                    .and_then(|()| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        });

        let session = Arc::new(PtySession {
            master: Mutex::new(Some(pair.master)),
            input: Mutex::new(Some(input)),
            killer: Mutex::new(child.clone_killer()),
        });

        let read_thread = thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => on_output(&buf[..n]),
                }
            }
        });

        let waiting = Arc::clone(&session);
        thread::spawn(move || {
            let exit = match child.wait() {
                Ok(status) => Exit {
                    code: Some(status.exit_code()),
                    error: None,
                },
                Err(e) => Exit {
                    code: None,
                    error: Some(format!("cannot get exit status: {e}")),
                },
            };
            // A pseudoconsole keeps its output pipe open after the process
            // exits; closing it flushes remaining output and ends the reader.
            waiting.input.lock().unwrap().take();
            waiting.master.lock().unwrap().take();
            let _ = read_thread.join();
            on_exit(exit);
        });

        Ok(session)
    }

    /// Queues `data` as terminal input.
    pub fn write(&self, data: &[u8]) -> Result<(), String> {
        let input = self.input.lock().unwrap();
        let input = input.as_ref().ok_or("the process has exited")?;
        input
            .send(data.to_vec())
            .map_err(|_| "terminal input is closed".to_string())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let master = self.master.lock().unwrap();
        let master = master.as_ref().ok_or("the process has exited")?;
        master
            .resize(size(cols, rows))
            .map_err(|e| format!("cannot resize terminal: {e}"))
    }

    /// Asks the OS to terminate the process. The outcome is observed through
    /// `on_exit`: portable-pty 0.9 inverts `TerminateProcess`'s result on
    /// Windows (success is reported as an error), so it is not used here.
    pub fn kill(&self) {
        let _ = self.killer.lock().unwrap().kill();
    }
}

/// Open sessions, keyed by an id handed to the frontend.
#[derive(Default)]
pub struct Terminals {
    next_id: AtomicU32,
    sessions: Mutex<HashMap<u32, Arc<PtySession>>>,
}

impl Terminals {
    /// Registers the session `start` creates under a fresh id. The map stays
    /// locked meanwhile, so a `remove` from a session that exits immediately
    /// waits until the session has been inserted.
    pub fn start(
        &self,
        start: impl FnOnce(u32) -> Result<Arc<PtySession>, String>,
    ) -> Result<u32, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(id, start(id)?);
        Ok(id)
    }

    pub fn remove(&self, id: u32) -> Option<Arc<PtySession>> {
        self.sessions.lock().unwrap().remove(&id)
    }

    pub fn get(&self, id: u32) -> Result<Arc<PtySession>, String> {
        self.sessions
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("terminal {id} is not running"))
    }
}

/// Arguments for an interactive `ssh` to `alias`. The alias is passed as one
/// argv element after `--`, so it can never be read as an option.
pub fn ssh_args(config: Option<&Path>, alias: &str) -> Vec<OsString> {
    let mut args = Vec::new();
    if let Some(config) = config {
        args.push("-F".into());
        args.push(config.into());
    }
    args.push("--".into());
    args.push(alias.into());
    args
}

pub fn ssh_command(ssh: &Path, config: Option<&Path>, alias: &str) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(ssh);
    cmd.args(ssh_args(config, alias));
    cmd.env("TERM", "xterm-256color");
    if let Some(home) = std::env::var_os("USERPROFILE") {
        cmd.cwd(home);
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn ssh_args_keep_alias_as_one_element_after_double_dash() {
        assert_eq!(ssh_args(None, "web prod"), ["--", "web prod"]);
        assert_eq!(
            ssh_args(None, "-oProxyCommand=x"),
            ["--", "-oProxyCommand=x"]
        );
        let config = Path::new(r"C:\Users\A B\my config");
        assert_eq!(
            ssh_args(Some(config), "alpha"),
            ["-F", r"C:\Users\A B\my config", "--", "alpha"]
        );
    }

    #[test]
    fn ssh_command_sets_term() {
        let cmd = ssh_command(Path::new("ssh.exe"), None, "alpha");
        assert_eq!(cmd.get_env("TERM").unwrap(), "xterm-256color");
        assert_eq!(cmd.get_argv()[1..], ["--", "alpha"].map(OsString::from));
    }

    // ConPTY asks for the cursor position (ESC[6n) on startup and waits for
    // the answer, which xterm.js sends in the app. Tests answer up front.
    const CURSOR_REPLY: &[u8] = b"\x1b[1;1R";

    struct Run {
        session: Arc<PtySession>,
        output: Arc<Mutex<Vec<u8>>>,
        exit: mpsc::Receiver<Exit>,
    }

    fn run(args: &[&str]) -> Run {
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(args);
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&output);
        let (tx, exit) = mpsc::channel();
        let session = PtySession::spawn(
            cmd,
            80,
            24,
            move |data| sink.lock().unwrap().extend_from_slice(data),
            move |e| tx.send(e).unwrap(),
        )
        .unwrap();
        session.write(CURSOR_REPLY).unwrap();
        Run {
            session,
            output,
            exit,
        }
    }

    fn wait(run: &Run) -> Exit {
        run.exit
            .recv_timeout(Duration::from_secs(15))
            .expect("process did not exit")
    }

    #[test]
    fn delivers_output_then_exit_code() {
        let r = run(&["/c", "echo hello-from-pty"]);
        assert_eq!(
            wait(&r),
            Exit {
                code: Some(0),
                error: None
            }
        );
        let out = String::from_utf8_lossy(&r.output.lock().unwrap()).into_owned();
        assert!(out.contains("hello-from-pty"), "{out:?}");
    }

    #[test]
    fn forwards_input_and_resizes() {
        let r = run(&["/q", "/k"]);
        r.session.resize(120, 40).unwrap();
        r.session.write(b"exit 3\r").unwrap();
        assert_eq!(wait(&r).code, Some(3));
        assert!(r.session.write(b"x").is_err());
        assert!(r.session.resize(80, 24).is_err());
    }

    #[test]
    fn immediate_exit_is_unregistered() {
        let terminals = Arc::new(Terminals::default());
        let (tx, exited) = mpsc::channel();
        let registry = Arc::clone(&terminals);
        let id = terminals
            .start(|id| {
                let mut cmd = CommandBuilder::new("cmd.exe");
                cmd.args(["/c", "exit 0"]);
                let session = PtySession::spawn(
                    cmd,
                    80,
                    24,
                    |_| {},
                    move |_| {
                        registry.remove(id);
                        tx.send(()).unwrap();
                    },
                )?;
                session.write(CURSOR_REPLY)?;
                Ok(session)
            })
            .unwrap();
        exited.recv_timeout(Duration::from_secs(15)).unwrap();
        assert!(terminals.get(id).is_err());
    }

    #[test]
    fn kill_ends_the_session() {
        let r = run(&["/q", "/k"]);
        r.session.kill();
        assert_ne!(wait(&r).code, Some(0));
    }
}
