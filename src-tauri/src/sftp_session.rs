//! Long-lived SFTP connections for the file pane, one per tab, so browsing a
//! folder is one request rather than a new connection. Windows `sftp.exe`
//! buffers its output when piped, so it cannot be driven interactively;
//! instead the app runs `ssh -s <alias> sftp` itself, as sftp.exe does, and
//! russh-sftp speaks the SFTP protocol over its stdin and stdout. OpenSSH
//! still handles config, authentication and host keys. Transfers keep using
//! sftp.exe batches ([`crate::transfer`]).

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::{Config, SftpSession};
use russh_sftp::protocol::{FileAttributes, StatusCode};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use crate::sftp::{classify, Entry, Error, ErrorKind, Kind, Listing};

/// Limit for connecting and logging in, not counting time spent in prompts.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Limit for each request on an open connection.
const REQUEST_TIMEOUT_SECS: u64 = 30;
/// Most of ssh's stderr kept for error messages.
const MAX_STDERR: usize = 16 * 1024;

/// `ssh` arguments that open the SFTP subsystem on `alias`, the same
/// overrides sftp.exe passes, so a `RemoteCommand` or `RequestTTY` meant for
/// interactive logins (e.g. starting Byobu) does not apply.
pub fn ssh_args(config: Option<&Path>, alias: &str) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = [
        "-oForwardX11=no",
        "-oPermitLocalCommand=no",
        "-oClearAllForwardings=yes",
        "-oForwardAgent=no",
        "-oRemoteCommand=none",
        "-oRequestTTY=no",
        "-s",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    if let Some(config) = config {
        args.push("-F".into());
        args.push(config.into());
    }
    args.extend(["--".into(), alias.into(), "sftp".into()]);
    args
}

/// One running `ssh -s ... sftp` process and the SFTP session over it.
pub struct Connection {
    alias: String,
    config: Option<PathBuf>,
    sftp: SftpSession,
    /// Killed when the connection is dropped.
    child: Mutex<Child>,
}

impl Connection {
    /// Starts `cmd`, whose stdin and stdout must carry the SFTP protocol, and
    /// initializes the session. Login prompts go through askpass meanwhile.
    pub async fn open(
        mut cmd: Command,
        alias: &str,
        config: Option<&Path>,
    ) -> Result<Connection, Error> {
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let program = cmd.as_std().get_program().to_string_lossy().into_owned();
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                Error::new(
                    ErrorKind::Unavailable,
                    format!("cannot start {program}: {e}"),
                )
            })?;
        let (Some(stdin), Some(stdout), Some(mut stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(Error::new(ErrorKind::Other, "ssh pipes are missing"));
        };
        // Drained throughout, so a chatty ssh never stalls; kept for errors.
        let log = tokio::spawn(async move {
            let mut text = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n @ 1..) = stderr.read(&mut chunk).await {
                if text.len() < MAX_STDERR {
                    text.extend_from_slice(&chunk[..n]);
                }
            }
            String::from_utf8_lossy(&text).into_owned()
        });
        // The protocol's own timeout would also count time spent typing a
        // password; [`pausable_timeout`] does not.
        let cfg = Config {
            request_timeout_secs: 24 * 60 * 60,
            ..Config::default()
        };
        let init = pausable_timeout(
            CONNECT_TIMEOUT,
            SftpSession::new_with_config(tokio::io::join(stdout, stdin), cfg),
        )
        .await;
        let sftp = match init {
            Some(Ok(sftp)) => sftp,
            failed => {
                // ssh exits after a failed login; its stderr says why.
                let _ = child.start_kill();
                let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
                let stderr = tokio::time::timeout(Duration::from_secs(2), log)
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or_default();
                return Err(match failed {
                    None if stderr.trim().is_empty() => Error::new(
                        ErrorKind::Timeout,
                        format!("no SFTP session within {} s", CONNECT_TIMEOUT.as_secs()),
                    ),
                    Some(Err(e)) if stderr.trim().is_empty() => {
                        Error::new(ErrorKind::Other, format!("SFTP session failed: {e}"))
                    }
                    _ => classify(&stderr, false),
                });
            }
        };
        sftp.set_timeout(REQUEST_TIMEOUT_SECS);
        Ok(Connection {
            alias: alias.to_string(),
            config: config.map(Path::to_path_buf),
            sftp,
            child: Mutex::new(child),
        })
    }

    /// False once ssh has exited, e.g. after the network dropped.
    fn alive(&self) -> bool {
        matches!(self.child.lock().unwrap().try_wait(), Ok(None))
    }

    /// Lists `path`, or the login directory, as an absolute path.
    pub async fn list(&self, path: Option<&str>) -> Result<Listing, Failure> {
        let path = path.filter(|p| !p.is_empty()).unwrap_or(".");
        let path = self
            .sftp
            .canonicalize(path)
            .await
            .map_err(|e| failure(path, e))?;
        let dir = self
            .sftp
            .read_dir(path.as_str())
            .await
            .map_err(|e| failure(&path, e))?;
        let entries = dir
            .map(|entry| entry_from(entry.file_name(), &entry.metadata()))
            .collect();
        Ok(Listing {
            path,
            entries,
            unparsed: Vec::new(),
        })
    }
}

/// A failed request, and whether the connection itself is unusable.
#[derive(Debug)]
pub struct Failure {
    pub error: Error,
    pub lost: bool,
}

fn failure(path: &str, e: SftpError) -> Failure {
    match e {
        SftpError::Status(status)
            if !matches!(
                status.status_code,
                StatusCode::NoConnection | StatusCode::ConnectionLost
            ) =>
        {
            let why = if status.error_message.is_empty() {
                status.status_code.to_string()
            } else {
                status.error_message
            };
            Failure {
                error: Error::new(ErrorKind::Remote, format!("{path}: {why}")),
                lost: false,
            }
        }
        SftpError::Timeout => Failure {
            error: Error::new(
                ErrorKind::Timeout,
                format!("{path}: no answer within {REQUEST_TIMEOUT_SECS} s"),
            ),
            lost: true,
        },
        e => Failure {
            error: Error::new(
                ErrorKind::Unreachable,
                format!("the SFTP connection was lost ({e})"),
            ),
            lost: true,
        },
    }
}

/// The type bits (S_IFMT) of the mode. Attributes from readdir and lstat
/// describe a link itself, not its target.
fn kind_of(attrs: &FileAttributes) -> Kind {
    match attrs.permissions.map(|p| p & 0o170_000) {
        Some(0o040_000) => Kind::Dir,
        Some(0o100_000) => Kind::File,
        Some(0o120_000) => Kind::Link,
        _ => Kind::Other,
    }
}

fn entry_from(name: String, attrs: &FileAttributes) -> Entry {
    Entry {
        name,
        kind: kind_of(attrs),
        size: attrs.size.unwrap_or(0),
        modified: attrs.mtime.map(format_time).unwrap_or_default(),
    }
}

/// `YYYY-MM-DD HH:MM UTC` from Unix seconds.
fn format_time(secs: u32) -> String {
    let days = i64::from(secs / 86_400);
    let minutes = secs % 86_400 / 60;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        minutes / 60,
        minutes % 60
    )
}

/// Awaits `fut` for at most `limit`, not counting time while a prompt waits
/// for the user ([`crate::process::pause_timeouts`]).
async fn pausable_timeout<F: Future>(limit: Duration, fut: F) -> Option<F::Output> {
    const TICK: Duration = Duration::from_millis(100);
    tokio::pin!(fut);
    let mut elapsed = Duration::ZERO;
    loop {
        tokio::select! {
            out = &mut fut => return Some(out),
            () = tokio::time::sleep(TICK) => {
                if !crate::process::timeouts_paused() {
                    elapsed += TICK;
                    if elapsed >= limit {
                        return None;
                    }
                }
            }
        }
    }
}

type Slot = Arc<tokio::sync::Mutex<Option<Arc<Connection>>>>;

/// Open connections, keyed by the tab's session id.
#[derive(Default)]
pub struct Connections(Mutex<HashMap<String, Slot>>);

impl Connections {
    /// The tab's connection, opened with `command` if there is none for
    /// this alias and config or the previous one has ended. Concurrent
    /// callers for the same tab wait for one connection.
    pub async fn get(
        &self,
        session: &str,
        alias: &str,
        config: Option<&Path>,
        command: impl FnOnce() -> Command,
    ) -> Result<Arc<Connection>, Error> {
        let slot = self
            .0
            .lock()
            .unwrap()
            .entry(session.to_string())
            .or_default()
            .clone();
        let mut slot = slot.lock().await;
        if let Some(connection) = slot.as_ref() {
            if connection.alias == alias
                && connection.config.as_deref() == config
                && connection.alive()
            {
                return Ok(connection.clone());
            }
        }
        *slot = None;
        let connection = Arc::new(Connection::open(command(), alias, config).await?);
        *slot = Some(connection.clone());
        Ok(connection)
    }

    /// Drops `connection` after it failed, so the next request reconnects.
    pub async fn discard(&self, session: &str, connection: &Arc<Connection>) {
        let slot = self.0.lock().unwrap().get(session).cloned();
        if let Some(slot) = slot {
            let mut slot = slot.lock().await;
            if slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, connection)) {
                *slot = None;
            }
        }
    }

    /// Ends the tab's connection, if any; ssh is killed once no request
    /// still uses it.
    pub fn close(&self, session: &str) {
        self.0.lock().unwrap().remove(session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_interactive_settings_and_passes_the_alias_last() {
        let args = ssh_args(Some(Path::new(r"C:\cfg dir\config")), "-oProxyCommand=x");
        let args: Vec<_> = args.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            args,
            [
                "-oForwardX11=no",
                "-oPermitLocalCommand=no",
                "-oClearAllForwardings=yes",
                "-oForwardAgent=no",
                "-oRemoteCommand=none",
                "-oRequestTTY=no",
                "-s",
                "-F",
                r"C:\cfg dir\config",
                "--",
                "-oProxyCommand=x",
                "sftp"
            ]
        );
    }

    #[test]
    fn maps_file_types_from_the_mode() {
        let attrs = |mode: u32| FileAttributes {
            permissions: Some(mode),
            size: Some(5),
            mtime: Some(0),
            ..FileAttributes::default()
        };
        let kind = |mode| entry_from("x".into(), &attrs(mode)).kind;
        assert_eq!(kind(0o040_755), Kind::Dir);
        assert_eq!(kind(0o100_644), Kind::File);
        // A link's mode shares bits with a regular file's.
        assert_eq!(kind(0o120_777), Kind::Link);
        assert_eq!(kind(0o140_777), Kind::Other);
        let entry = entry_from("a b.txt".into(), &attrs(0o100_644));
        assert_eq!(entry.size, 5);
        assert_eq!(entry.modified, "1970-01-01 00:00 UTC");
        let bare = entry_from("x".into(), &FileAttributes::default());
        assert_eq!((bare.kind, bare.modified.as_str()), (Kind::Other, ""));
    }

    #[test]
    fn formats_times_as_utc_dates() {
        assert_eq!(format_time(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(format_time(1_792_576_140), "2026-10-21 09:49 UTC");
        assert_eq!(format_time(u32::MAX), "2106-02-07 06:28 UTC");
    }

    #[test]
    fn separates_operation_errors_from_lost_connections() {
        let status = |code| {
            SftpError::Status(russh_sftp::protocol::Status {
                id: 1,
                status_code: code,
                error_message: String::new(),
                language_tag: String::new(),
            })
        };
        let denied = failure("/root", status(StatusCode::PermissionDenied));
        assert!(!denied.lost);
        assert_eq!(
            denied.error,
            Error::new(ErrorKind::Remote, "/root: Permission denied")
        );
        assert!(failure("/x", status(StatusCode::ConnectionLost)).lost);
        assert!(failure("/x", SftpError::IO("broken pipe".into())).lost);
        let timeout = failure("/x", SftpError::Timeout);
        assert!(timeout.lost);
        assert_eq!(timeout.error.kind, ErrorKind::Timeout);
    }

    #[test]
    fn pausable_timeout_expires_and_returns_results() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let slow = tokio::time::sleep(Duration::from_secs(30));
            assert!(pausable_timeout(Duration::from_millis(200), slow)
                .await
                .is_none());
            assert_eq!(
                pausable_timeout(Duration::from_secs(5), async { 7 }).await,
                Some(7)
            );
        });
    }
}

#[cfg(all(test, windows))]
mod local_server_tests {
    use super::*;

    /// Git for Windows' sftp-server speaks SFTP on stdio, like `ssh -s`.
    fn server(root: &Path) -> Option<Command> {
        let server = r"C:\Program Files\Git\usr\lib\ssh\sftp-server.exe";
        if !Path::new(server).exists() {
            eprintln!("skipped: {server} not found");
            return None;
        }
        let mut cmd = Command::new(server);
        // The msys sftp-server loads its DLLs from Git's usr\bin.
        cmd.current_dir(root).env(
            "PATH",
            format!(
                r"C:\Program Files\Git\usr\bin;{}",
                std::env::var("PATH").unwrap_or_default()
            ),
        );
        Some(cmd)
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn names(listing: &Listing) -> Vec<&str> {
        let mut names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        names.sort();
        names
    }

    #[test]
    fn browses_folders_over_one_connection() {
        let dir = tempfile::tempdir().unwrap();
        let Some(_) = server(dir.path()) else {
            return;
        };
        let sub = dir.path().join("it's [a] dir");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("máy chủ.txt"), "12345").unwrap();
        std::fs::write(dir.path().join("a b.txt"), "x").unwrap();

        runtime().block_on(async {
            let connections = Connections::default();
            let get = || connections.get("tab", "local", None, || server(dir.path()).unwrap());
            let connection = get().await.unwrap();
            let root = connection.list(None).await.unwrap();
            assert_eq!(names(&root), ["a b.txt", "it's [a] dir"]);
            let inner = connection
                .list(Some(&format!("{}/it's [a] dir", root.path)))
                .await
                .unwrap();
            assert!(inner.path.ends_with("/it's [a] dir"), "{}", inner.path);
            assert_eq!(names(&inner), ["máy chủ.txt"]);
            assert_eq!(inner.entries[0].size, 5);
            assert_eq!(inner.entries[0].kind, Kind::File);

            let missing = connection.list(Some("/no/such/dir")).await.unwrap_err();
            assert!(!missing.lost);
            assert_eq!(missing.error.kind, ErrorKind::Remote);

            // Still the same process after browsing and an error.
            assert!(Arc::ptr_eq(&connection, &get().await.unwrap()));

            // A dead process is replaced on the next request.
            connection.child.lock().unwrap().start_kill().unwrap();
            while connection.alive() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let replaced = get().await.unwrap();
            assert!(!Arc::ptr_eq(&connection, &replaced));
            assert_eq!(names(&replaced.list(None).await.unwrap()).len(), 2);
        });
    }

    #[test]
    fn reports_a_process_that_is_not_an_sftp_server() {
        runtime().block_on(async {
            let mut cmd = Command::new("cmd");
            cmd.args(["/c", "echo ssh: Could not resolve hostname nowhere 1>&2"]);
            let err = Connection::open(cmd, "nowhere", None).await.err().unwrap();
            assert_eq!(err.kind, ErrorKind::Unreachable, "{}", err.message);
            assert_eq!(err.message, "ssh: Could not resolve hostname nowhere");
        });
    }
}
