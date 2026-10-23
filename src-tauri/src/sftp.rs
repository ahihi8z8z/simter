//! Remote file operations through the system `sftp` client in batch mode
//! (`sftp -b -`), one process per operation, so OpenSSH applies the same
//! config (`ProxyJump`, `IdentityFile`, ports) as the terminal.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde::Serialize;

use crate::askpass::Askpass;
use crate::process::{self, RunError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Dir,
    File,
    Link,
    Other,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub size: u64,
    /// As sftp prints it, e.g. `Oct 21 10:29` or `Jan  3  2024`.
    pub modified: String,
}

/// How much a transfer moves: bytes, and for a folder the regular files in
/// it (`None` for a single file).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Size {
    pub bytes: u64,
    pub files: Option<u64>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub path: String,
    pub entries: Vec<Entry>,
    /// Listing lines that could not be parsed (reported, not hidden).
    pub unparsed: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    /// sftp.exe missing or could not be started.
    Unavailable,
    /// The SSH config could not be parsed.
    Config,
    Unreachable,
    /// Unknown or changed host key; never accepted automatically here.
    HostKey,
    Auth,
    /// The server does not offer the SFTP subsystem.
    NoSftp,
    /// Connected, but the operation failed (missing path, permissions...).
    Remote,
    /// The path cannot be expressed in an sftp batch command.
    UnsupportedPath,
    Timeout,
    /// The transfer destination exists and overwriting was not confirmed.
    Exists,
    Cancelled,
    /// A local file or folder could not be read or written.
    Local,
    Other,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Error {
            kind,
            message: message.into(),
        }
    }
}

/// Quotes `path` as one sftp batch argument. Windows sftp turns `\` into `/`
/// before parsing, so backslash escapes are unavailable; the quote character
/// is chosen so the path never needs one.
pub fn quote(path: &str) -> Result<String, Error> {
    let unsupported = |why: &str| {
        Err(Error::new(
            ErrorKind::UnsupportedPath,
            format!("cannot use {path:?}: {why}"),
        ))
    };
    if path.contains('\\') {
        return unsupported("Windows sftp rewrites backslashes in remote paths");
    }
    if path.contains(['\n', '\r']) {
        return unsupported("line breaks cannot be passed to sftp");
    }
    if !path.contains('"') {
        Ok(format!("\"{path}\""))
    } else if !path.contains('\'') {
        Ok(format!("'{path}'"))
    } else {
        unsupported("it contains both single and double quotes")
    }
}

/// Batch script that changes to `path` (or stays in the login directory)
/// and lists it. `ls` without an argument lists the working directory
/// without glob expansion; `cd` never globs.
pub fn list_script(path: Option<&str>) -> Result<String, Error> {
    let mut script = String::new();
    if let Some(path) = path.filter(|p| !p.is_empty()) {
        script.push_str(&format!("cd {}\n", quote(path)?));
    }
    script.push_str("pwd\nls -lan\n");
    Ok(script)
}

/// Parses the output of [`list_script`]. With `-n`, sftp formats long
/// listings client-side, so the layout depends only on the local OpenSSH.
pub fn parse_listing(stdout: &str) -> Result<Listing, Error> {
    let mut path = None;
    let mut entries = Vec::new();
    let mut unparsed = Vec::new();
    for line in stdout.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with("sftp> ") {
            continue;
        }
        if let Some(cwd) = line.strip_prefix("Remote working directory: ") {
            path = Some(cwd.to_string());
        } else if let Some(entry) = parse_ls_line(line) {
            if entry.name != "." && entry.name != ".." {
                entries.push(entry);
            }
        } else {
            unparsed.push(line.to_string());
        }
    }
    let path = path
        .ok_or_else(|| Error::new(ErrorKind::Other, "sftp did not report the remote directory"))?;
    Ok(Listing {
        path,
        entries,
        unparsed,
    })
}

/// Parses `mode nlink uid gid size month day time-or-year name`. The name is
/// everything after the single space following the date, so leading and
/// repeated spaces in names survive.
fn parse_ls_line(line: &str) -> Option<Entry> {
    let mut rest = line;
    let mut fields = [""; 8];
    for field in &mut fields {
        rest = rest.trim_start_matches(' ');
        let end = rest.find(' ')?;
        *field = &rest[..end];
        rest = &rest[end..];
    }
    let name = rest.strip_prefix(' ').filter(|n| !n.is_empty())?;
    let [mode, _nlink, _uid, _gid, size, month, day, time] = fields;
    let kind = match mode.chars().next()? {
        'd' => Kind::Dir,
        '-' => Kind::File,
        'l' => Kind::Link,
        'c' | 'b' | 'p' | 's' => Kind::Other,
        _ => return None,
    };
    if mode.len() < 10 || !month.chars().all(char::is_alphabetic) {
        return None;
    }
    Some(Entry {
        name: name.to_string(),
        kind,
        size: size.parse().ok()?,
        modified: format!("{month} {day} {time}"),
    })
}

/// Classifies a failed sftp run. `connected` means sftp started executing
/// the batch (it echoes each command), so the failure is the operation's.
pub fn classify(stderr: &str, connected: bool) -> Error {
    let message = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "Connection closed")
        .collect::<Vec<_>>()
        .join("\n");
    let has = |needles: &[&str]| needles.iter().any(|n| stderr.contains(n));
    let kind = if connected {
        ErrorKind::Remote
    } else if has(&[
        "Host key verification failed",
        "REMOTE HOST IDENTIFICATION HAS CHANGED",
    ]) {
        ErrorKind::HostKey
    } else if has(&[
        "Permission denied (",
        "Too many authentication failures",
        "No more authentication methods",
    ]) {
        ErrorKind::Auth
    } else if has(&["subsystem request failed"]) {
        ErrorKind::NoSftp
    } else if has(&[
        "Could not resolve hostname",
        "Connection refused",
        "Connection timed out",
        "No route to host",
        "Network is unreachable",
        "Connection reset",
        "Operation timed out",
    ]) {
        ErrorKind::Unreachable
    } else if has(&[
        "Bad configuration option",
        "Terminating, ",
        "garbage at end of line",
        "Can't open user config file",
        "Bad owner or permissions",
    ]) {
        ErrorKind::Config
    } else {
        ErrorKind::Other
    };
    let message = if message.is_empty() {
        "sftp failed without an error message".to_string()
    } else {
        message
    };
    Error::new(kind, message)
}

/// Builds a fresh sftp process connected to the host, one per batch.
pub type Connect<'a> = &'a dyn Fn() -> Command;

pub fn sftp_command(
    sftp: &Path,
    config: Option<&Path>,
    alias: &str,
    askpass: Option<&Askpass>,
) -> Command {
    let mut cmd = Command::new(sftp);
    match askpass {
        // `-b` turns BatchMode on for ssh; an earlier value wins, so prompts
        // reach the askpass helper.
        Some(askpass) => {
            cmd.args(["-o", "BatchMode=no", "-b", "-"]);
            askpass.apply(&mut cmd);
        }
        // No way to answer password or host-key prompts.
        None => {
            cmd.args(["-b", "-", "-o", "BatchMode=yes"]);
        }
    }
    if let Some(config) = config {
        cmd.arg("-F").arg(config);
    }
    cmd.arg("--").arg(alias);
    cmd
}

/// Runs one batch `script` and returns its stdout, or the classified error.
pub fn run_batch(
    connect: Connect,
    script: &str,
    timeout: Option<Duration>,
    cancel: Option<&AtomicBool>,
) -> Result<String, Error> {
    let output =
        process::run_with(connect(), script.as_bytes(), timeout, cancel).map_err(|e| match e {
            RunError::Failed(m) => Error::new(ErrorKind::Unavailable, m),
            RunError::TimedOut(m) => Error::new(ErrorKind::Timeout, m),
            RunError::Cancelled => Error::new(ErrorKind::Cancelled, "Cancelled."),
        })?;
    if output.code != Some(0) {
        return Err(classify(&output.stderr, output.stdout.contains("sftp> ")));
    }
    Ok(output.stdout)
}

/// Lists `path`, or the login directory.
pub fn list(connect: Connect, path: Option<&str>, timeout: Duration) -> Result<Listing, Error> {
    parse_listing(&run_batch(
        connect,
        &list_script(path)?,
        Some(timeout),
        None,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    /// Captured from Windows `sftp.exe` (OpenSSH_for_Windows_9.5p2).
    const LISTING: &str = "sftp> cd \"/srv/data\"\r\n\
        sftp> pwd\r\n\
        Remote working directory: /srv/data\r\n\
        sftp> ls -lan\r\n\
        -rw-******    ? 1001     513             0 Oct 21 10:29   lead\r\n\
        drwx******    ? 1001     513             0 Oct 21 10:29 .\r\n\
        drwx******    ? 1001     513             0 Oct 21 10:29 ..\r\n\
        -rw-******    ? 1001     513             5 Oct 21 10:29 a file.txt\r\n\
        -rw-******    ? 1001     513             1 Oct 21 10:29 máy chủ.txt\r\n\
        drwx******    ? 1001     513             0 Oct 21 10:29 sub dir\r\n";

    fn names(listing: &Listing) -> Vec<&str> {
        listing.entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn parses_names_with_spaces_and_unicode() {
        let listing = parse_listing(LISTING).unwrap();
        assert_eq!(listing.path, "/srv/data");
        assert_eq!(
            names(&listing),
            ["  lead", "a file.txt", "máy chủ.txt", "sub dir"]
        );
        assert_eq!(
            listing.entries[1],
            Entry {
                name: "a file.txt".into(),
                kind: Kind::File,
                size: 5,
                modified: "Oct 21 10:29".into(),
            }
        );
        assert_eq!(listing.entries[3].kind, Kind::Dir);
        assert!(listing.unparsed.is_empty());
    }

    #[test]
    fn parses_links_years_and_large_sizes() {
        let out = "Remote working directory: /\n\
            lrwxrwxrwx    1 0        0                   7 Jan  3  2024 bin\n\
            -rw-r--r--    1 1000     1000      5368709120 Feb 29 23:59 disk.img\n\
            srwxrwxrwx    1 0        0                   0 Oct  1 08:00 sock\n";
        let listing = parse_listing(out).unwrap();
        assert_eq!(listing.entries[0].kind, Kind::Link);
        assert_eq!(listing.entries[0].modified, "Jan 3 2024");
        assert_eq!(listing.entries[1].size, 5_368_709_120);
        assert_eq!(listing.entries[2].kind, Kind::Other);
    }

    #[test]
    fn reports_unparsed_lines_instead_of_dropping_them() {
        let out = "Remote working directory: /x\nsomething unexpected\n";
        let listing = parse_listing(out).unwrap();
        assert_eq!(listing.unparsed, ["something unexpected"]);
    }

    #[test]
    fn requires_the_working_directory() {
        let err = parse_listing("sftp> ls -lan\n").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Other);
    }

    #[test]
    fn quotes_paths_without_escapes() {
        assert_eq!(quote("/srv/it's [x]").unwrap(), "\"/srv/it's [x]\"");
        assert_eq!(quote("/srv/say \"hi\"").unwrap(), "'/srv/say \"hi\"'");
        for bad in ["/a\\b", "/a\nb", "/it's \"x\""] {
            assert_eq!(quote(bad).unwrap_err().kind, ErrorKind::UnsupportedPath);
        }
    }

    #[test]
    fn builds_list_scripts() {
        assert_eq!(list_script(None).unwrap(), "pwd\nls -lan\n");
        assert_eq!(list_script(Some("")).unwrap(), "pwd\nls -lan\n");
        assert_eq!(
            list_script(Some("/home/me/my dir")).unwrap(),
            "cd \"/home/me/my dir\"\npwd\nls -lan\n"
        );
    }

    #[test]
    fn builds_argv_without_a_shell() {
        let cmd = sftp_command(
            Path::new("sftp.exe"),
            Some(Path::new(r"C:\cfg dir\config")),
            "-oProxyCommand=x",
            None,
        );
        let args: Vec<&OsStr> = cmd.get_args().collect();
        assert_eq!(
            args,
            [
                "-b",
                "-",
                "-o",
                "BatchMode=yes",
                "-F",
                r"C:\cfg dir\config",
                "--",
                "-oProxyCommand=x"
            ]
        );
    }

    #[test]
    fn classifies_connection_failures() {
        let cases = [
            (
                "ssh: connect to host 127.0.0.1 port 1: Connection refused\r\nConnection closed\r\n",
                ErrorKind::Unreachable,
            ),
            (
                "ssh: Could not resolve hostname x.invalid: No such host is known. \r\nConnection closed\r\n",
                ErrorKind::Unreachable,
            ),
            (
                "deploy@10.0.0.5: Permission denied (publickey,password).\r\nConnection closed\r\n",
                ErrorKind::Auth,
            ),
            (
                "No ED25519 host key is known for x and you have requested strict checking.\r\nHost key verification failed.\r\n",
                ErrorKind::HostKey,
            ),
            (
                "subsystem request failed on channel 0\r\nConnection closed\r\n",
                ErrorKind::NoSftp,
            ),
            (
                "C:\\Users\\me/.ssh/config: line 3: Bad configuration option: nosuchoption\r\n",
                ErrorKind::Config,
            ),
        ];
        for (stderr, kind) in cases {
            assert_eq!(classify(stderr, false).kind, kind, "{stderr}");
        }
        let err = classify(
            "ssh: connect to host 127.0.0.1 port 1: Connection refused\r\nConnection closed\r\n",
            false,
        );
        assert_eq!(
            err.message,
            "ssh: connect to host 127.0.0.1 port 1: Connection refused"
        );
    }

    #[test]
    fn classifies_failures_after_connecting_as_remote() {
        let err = classify("stat remote: No such file or directory\r\n", true);
        assert_eq!(
            err,
            Error::new(ErrorKind::Remote, "stat remote: No such file or directory")
        );
    }

    /// Checks quoting and parsing end to end with the real sftp.exe.
    #[cfg(windows)]
    #[test]
    fn lists_through_the_system_sftp_and_a_local_server() {
        let dir = tempfile::tempdir().unwrap();
        let Some(connect) = testing::local_server(dir.path()) else {
            return;
        };
        let sub = dir.path().join("it's [a] dir");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("máy chủ.txt"), "12345").unwrap();

        let run_list = |path: Option<&str>| list(&connect, path, Duration::from_secs(20)).unwrap();
        let root = run_list(None);
        assert_eq!(names(&root), ["it's [a] dir"]);
        assert_eq!(root.entries[0].kind, Kind::Dir);

        let inner = run_list(Some(&format!("{}/it's [a] dir", root.path)));
        assert!(inner.path.ends_with("/it's [a] dir"), "{}", inner.path);
        assert_eq!(names(&inner), ["máy chủ.txt"]);
        assert_eq!(inner.entries[0].size, 5);
    }

    /// Real sftp.exe against an unreachable alias; needs no network.
    #[cfg(windows)]
    #[test]
    fn reports_unreachable_hosts_with_the_system_sftp() {
        let sftp = crate::ssh_bin::locate("sftp.exe").expect("OpenSSH client installed");
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        std::fs::write(&config, "Host closed\n  HostName 127.0.0.1\n  Port 1\n").unwrap();
        let connect = || sftp_command(&sftp, Some(&config), "closed", None);
        let err = list(&connect, None, Duration::from_secs(20)).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Unreachable, "{}", err.message);
    }
}

#[cfg(all(test, windows))]
pub(crate) mod testing {
    use std::path::Path;
    use std::process::Command;

    /// Real sftp.exe talking to Git for Windows' sftp-server over `-D` (no
    /// ssh), serving `root` as the remote working directory. `None`, with a
    /// note, when Git's server is not installed.
    pub fn local_server(root: &Path) -> Option<impl Fn() -> Command> {
        // 8.3 path: sftp -D splits its argument on spaces.
        let server = r"C:\PROGRA~1\Git\usr\lib\ssh\sftp-server.exe";
        if !Path::new(server).exists() {
            eprintln!("skipped: {server} not found");
            return None;
        }
        let sftp = crate::ssh_bin::locate("sftp.exe").expect("OpenSSH client installed");
        let root = root.to_path_buf();
        // The msys sftp-server loads its DLLs from Git's usr\bin.
        let env_path = format!(
            r"C:\PROGRA~1\Git\usr\bin;{}",
            std::env::var("PATH").unwrap_or_default()
        );
        Some(move || {
            let mut cmd = Command::new(&sftp);
            cmd.args(["-b", "-", "-D", server])
                .current_dir(&root)
                .env("PATH", &env_path);
            cmd
        })
    }
}
