//! Locates the OpenSSH client executables.
//!
//! The Windows OpenSSH client in System32 is preferred. Other `ssh.exe` builds
//! commonly found on PATH (such as Git for Windows' MSYS build) resolve `~` and
//! config paths differently.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Finds `name` (e.g. `"ssh.exe"`) in `%WINDIR%\System32\OpenSSH`, then on PATH.
pub fn locate(name: &str) -> Result<PathBuf, String> {
    locate_in(name, std::env::var_os("WINDIR"), std::env::var_os("PATH"))
}

fn locate_in(
    name: &str,
    windir: Option<OsString>,
    path: Option<OsString>,
) -> Result<PathBuf, String> {
    let system = windir.map(|w| Path::new(&w).join("System32").join("OpenSSH").join(name));
    let on_path = path
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(name));
    system
        .into_iter()
        .chain(on_path)
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            format!(
                "{name} was not found. Install the Windows OpenSSH client \
                 (Settings > System > Optional features > OpenSSH Client)."
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn prefers_system32_openssh_over_path() {
        let windir = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        let system = windir
            .path()
            .join("System32")
            .join("OpenSSH")
            .join("ssh.exe");
        touch(&system);
        touch(&other.path().join("ssh.exe"));
        let path = std::env::join_paths([other.path()]).unwrap();
        let found = locate_in("ssh.exe", Some(windir.path().into()), Some(path)).unwrap();
        assert_eq!(found, system);
    }

    #[test]
    fn falls_back_to_path() {
        let windir = TempDir::new().unwrap();
        let first = TempDir::new().unwrap();
        let second = TempDir::new().unwrap();
        touch(&second.path().join("sftp.exe"));
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();
        let found = locate_in("sftp.exe", Some(windir.path().into()), Some(path)).unwrap();
        assert_eq!(found, second.path().join("sftp.exe"));
    }

    #[test]
    fn reports_missing_client() {
        let err = locate_in("ssh.exe", None, None).unwrap_err();
        assert!(err.contains("OpenSSH Client"), "{err}");
    }
}
