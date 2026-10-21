//! Effective connection details from `ssh -G <alias>`, which applies the full
//! OpenSSH config semantics (`Match`, wildcards, defaults) that the lexical
//! host scan does not.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde::Serialize;

use crate::process;

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct Details {
    pub user: Option<String>,
    pub hostname: Option<String>,
    pub port: Option<u16>,
}

/// Picks `user`, `hostname` and `port` out of `ssh -G` output
/// (one lowercase `keyword value` pair per line).
pub fn parse(output: &str) -> Details {
    let mut details = Details::default();
    for line in output.lines() {
        let Some((key, value)) = line.trim().split_once(' ') else {
            continue;
        };
        let value = value.trim();
        match key {
            "user" => details.user = Some(value.to_string()),
            "hostname" => details.hostname = Some(value.to_string()),
            "port" => details.port = value.parse().ok(),
            _ => {}
        }
    }
    details
}

pub fn ssh_g_command(ssh: &Path, config: Option<&Path>, alias: &str) -> Command {
    let mut cmd = Command::new(ssh);
    cmd.arg("-G");
    if let Some(config) = config {
        cmd.arg("-F").arg(config);
    }
    cmd.arg("--").arg(alias);
    cmd
}

/// Runs `ssh -G` and parses its output. ssh may run `Match exec` commands or
/// canonicalize names via DNS while resolving, hence the timeout.
pub fn resolve(
    ssh: &Path,
    config: Option<&Path>,
    alias: &str,
    timeout: Duration,
) -> Result<Details, String> {
    let output =
        process::run(ssh_g_command(ssh, config, alias), b"", timeout).map_err(|e| e.to_string())?;
    if output.code != Some(0) {
        let stderr = output.stderr.trim();
        return Err(if stderr.is_empty() {
            "ssh -G failed without an error message".to_string()
        } else {
            format!("ssh -G failed: {stderr}")
        });
    }
    Ok(parse(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn parses_user_hostname_and_port() {
        let out = "user deploy\nhostname 10.0.0.5\nport 2222\nproxyjump bastion\nidentityfile ~/.ssh/id_ed25519\n";
        assert_eq!(
            parse(out),
            Details {
                user: Some("deploy".into()),
                hostname: Some("10.0.0.5".into()),
                port: Some(2222),
            }
        );
    }

    #[test]
    fn handles_crlf_and_missing_or_bad_values() {
        let out = "hostname example.com\r\nport nope\r\nbatchmode no\r\n";
        assert_eq!(
            parse(out),
            Details {
                user: None,
                hostname: Some("example.com".into()),
                port: None,
            }
        );
    }

    #[test]
    fn builds_argv_without_a_shell() {
        let cmd = ssh_g_command(
            Path::new("ssh.exe"),
            Some(Path::new(r"C:\My Configs\ssh config")),
            "-oProxyCommand=x",
        );
        let args: Vec<&OsStr> = cmd.get_args().collect();
        assert_eq!(
            args,
            [
                "-G",
                "-F",
                r"C:\My Configs\ssh config",
                "--",
                "-oProxyCommand=x"
            ]
        );
    }

    /// Exercises the real OpenSSH client; needs no network.
    #[cfg(windows)]
    #[test]
    fn resolves_with_the_system_ssh() {
        let ssh = crate::ssh_bin::locate("ssh.exe").expect("OpenSSH client installed");
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config with space");
        std::fs::write(
            &config,
            "Host web\n  HostName 192.0.2.10\n  User deploy\n  Port 2200\n",
        )
        .unwrap();
        let timeout = Duration::from_secs(10);
        let details = resolve(&ssh, Some(&config), "web", timeout).unwrap();
        assert_eq!(details.hostname.as_deref(), Some("192.0.2.10"));
        assert_eq!(details.user.as_deref(), Some("deploy"));
        assert_eq!(details.port, Some(2200));

        std::fs::write(&config, "Host web\n  NoSuchOption yes\n").unwrap();
        let err = resolve(&ssh, Some(&config), "web", timeout).unwrap_err();
        assert!(err.contains("ssh -G failed"), "{err}");
    }
}
