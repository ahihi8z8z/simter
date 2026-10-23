//! Uploads and downloads of files and folders through sftp batch runs.
//! Destinations are checked first and never replaced unless the caller
//! passes `overwrite`. Downloads land under a temporary name and are moved
//! into place only once complete.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::sftp::{self, quote, Connect, Error, ErrorKind, Size};

/// Limit for the destination check listing; transfers themselves have none.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// Cancel flags of running transfers, keyed by the frontend's transfer id.
#[derive(Default)]
pub struct Transfers(Mutex<HashMap<u32, Arc<AtomicBool>>>);

impl Transfers {
    pub fn begin(&self, id: u32) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.0.lock().unwrap().insert(id, flag.clone());
        flag
    }

    pub fn end(&self, id: u32) {
        self.0.lock().unwrap().remove(&id);
    }

    /// Does nothing if the transfer already finished.
    pub fn cancel(&self, id: u32) {
        if let Some(flag) = self.0.lock().unwrap().get(&id) {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

/// Uploads the file or folder `local` into the remote folder `remote_dir`.
pub fn upload(
    connect: Connect,
    local: &Path,
    remote_dir: &str,
    overwrite: bool,
    cancel: &AtomicBool,
) -> Result<(), Error> {
    let meta = fs::metadata(local).map_err(|e| local_error(local, &e))?;
    let (Some(parent), Some(name)) = (local.parent(), local.file_name().and_then(|n| n.to_str()))
    else {
        return Err(Error::new(
            ErrorKind::UnsupportedPath,
            format!("cannot upload {}", local.display()),
        ));
    };
    if !overwrite {
        let listing = sftp::list(connect, Some(remote_dir), CHECK_TIMEOUT)?;
        // An unparsed line ending in the name may be the same entry; treat
        // it as a conflict rather than risk replacing it.
        let suffix = format!(" {name}");
        if listing.entries.iter().any(|e| e.name == name)
            || listing.unparsed.iter().any(|l| l.ends_with(&suffix))
        {
            return Err(Error::new(
                ErrorKind::Exists,
                format!("“{name}” already exists in {}", listing.path),
            ));
        }
    }
    let script = upload_script(&local_arg(parent)?, remote_dir, name, meta.is_dir())?;
    sftp::run_batch(connect, &script, None, Some(cancel))
        .map(drop)
        .map_err(|e| match e.kind {
            ErrorKind::Cancelled => Error::new(
                ErrorKind::Cancelled,
                "Upload cancelled; a partial copy may remain on the server.",
            ),
            _ => e,
        })
}

/// Downloads `name` from the remote folder `remote_dir` into `local_dir`.
/// `tag` makes the temporary name unique among concurrent transfers.
pub fn download(
    connect: Connect,
    remote_dir: &str,
    name: &str,
    local_dir: &Path,
    overwrite: bool,
    tag: &str,
    cancel: &AtomicBool,
) -> Result<(), Error> {
    check_local_name(name)?;
    let target = local_dir.join(name);
    let exists = || fs::symlink_metadata(&target).is_ok();
    let already_exists = || {
        Error::new(
            ErrorKind::Exists,
            format!("“{name}” already exists in {}", local_dir.display()),
        )
    };
    if !overwrite && exists() {
        return Err(already_exists());
    }
    let temp_name = temp_name(name, tag);
    let temp = local_dir.join(&temp_name);
    let script = download_script(&local_arg(local_dir)?, remote_dir, name, &temp_name)?;
    if let Err(e) = sftp::run_batch(connect, &script, None, Some(cancel)) {
        remove_partial(&temp);
        return Err(match e.kind {
            ErrorKind::Cancelled => Error::new(ErrorKind::Cancelled, "Download cancelled."),
            _ => e,
        });
    }
    // Something may have appeared at the target while downloading; keep the
    // finished copy under its temporary name rather than replace it.
    if !overwrite && exists() {
        let mut err = already_exists();
        err.message += &format!("; the download was kept as {temp_name}");
        return Err(err);
    }
    place(&temp, &target).map_err(|e| {
        Error::new(
            ErrorKind::Local,
            format!(
                "cannot move {temp_name} to {}: {e}; the download was kept as {temp_name}",
                target.display()
            ),
        )
    })
}

/// Where [`download`] writes `name` until it is complete.
pub fn temp_name(name: &str, tag: &str) -> String {
    format!(".{name}.{tag}.part")
}

/// Size of the local file or folder `path`, for progress. Links are not
/// followed; anything missing or unreadable counts as nothing.
pub fn local_size(path: &Path) -> Size {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            let mut size = Size {
                bytes: 0,
                files: Some(0),
            };
            let mut pending = vec![path.to_path_buf()];
            while let Some(dir) = pending.pop() {
                for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                    match entry.metadata() {
                        Ok(meta) if meta.is_dir() => pending.push(entry.path()),
                        Ok(meta) if meta.is_file() => {
                            size.bytes += meta.len();
                            size.files = size.files.map(|n| n + 1);
                        }
                        _ => {}
                    }
                }
            }
            size
        }
        Ok(meta) if meta.is_file() => Size {
            bytes: meta.len(),
            files: None,
        },
        _ => Size::default(),
    }
}

/// `cd`/`lcd` never glob, and sftp escapes glob characters inside quotes,
/// so names are taken literally. `put -r` needs the remote folder to exist
/// first; `-mkdir` ignores the error when it already does.
pub fn upload_script(
    local_parent: &str,
    remote_dir: &str,
    name: &str,
    dir: bool,
) -> Result<String, Error> {
    let name = quote(name)?;
    let mut script = format!("lcd {}\ncd {}\n", quote(local_parent)?, quote(remote_dir)?);
    if dir {
        script.push_str(&format!("-mkdir {name}\nput -r {name}\n"));
    } else {
        script.push_str(&format!("put {name}\n"));
    }
    Ok(script)
}

pub fn download_script(
    local_dir: &str,
    remote_dir: &str,
    name: &str,
    temp_name: &str,
) -> Result<String, Error> {
    Ok(format!(
        "lcd {}\ncd {}\nget -r {} {}\n",
        quote(local_dir)?,
        quote(remote_dir)?,
        quote(name)?,
        quote(temp_name)?
    ))
}

/// A local path as sftp accepts it: Windows sftp reads `/` as a separator,
/// while a `\` would be rejected by [`quote`].
fn local_arg(path: &Path) -> Result<String, Error> {
    path.to_str().map(|p| p.replace('\\', "/")).ok_or_else(|| {
        Error::new(
            ErrorKind::UnsupportedPath,
            format!("{} is not valid Unicode", path.display()),
        )
    })
}

/// Rejects remote names that cannot be created on Windows, before
/// transferring anything.
pub fn check_local_name(name: &str) -> Result<(), Error> {
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let numbered = (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && stem.as_bytes()[3].is_ascii_digit();
    let why = if name.is_empty() || name == "." || name == ".." {
        Some("it is not a file name")
    } else if name
        .chars()
        .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        Some("it contains a character Windows does not allow (< > : \" / \\ | ? *)")
    } else if name.ends_with(['.', ' ']) {
        Some("Windows does not allow names ending in a dot or space")
    } else if RESERVED.contains(&stem.as_str()) || numbered {
        Some("it is a reserved device name on Windows")
    } else {
        None
    };
    match why {
        Some(why) => Err(Error::new(
            ErrorKind::UnsupportedPath,
            format!("cannot download “{name}”: {why}"),
        )),
        None => Ok(()),
    }
}

/// Moves a finished download into place. A folder is merged into an
/// existing folder (the overwrite the user confirmed); files replace files.
fn place(temp: &Path, target: &Path) -> io::Result<()> {
    if temp.is_dir() && target.is_dir() {
        for entry in fs::read_dir(temp)? {
            let entry = entry?;
            place(&entry.path(), &target.join(entry.file_name()))?;
        }
        fs::remove_dir(temp)
    } else if target.is_dir() {
        Err(io::Error::other("a folder with that name exists"))
    } else {
        // Replaces an existing file (MoveFileEx with REPLACE_EXISTING).
        fs::rename(temp, target)
    }
}

/// Removes this transfer's own temporary copy after a failure.
fn remove_partial(temp: &Path) {
    let _ = if temp.is_dir() {
        fs::remove_dir_all(temp)
    } else {
        fs::remove_file(temp)
    };
}

fn local_error(path: &Path, e: &io::Error) -> Error {
    Error::new(ErrorKind::Local, format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_upload_scripts() {
        assert_eq!(
            upload_script("C:/Users/me/My Docs", "/srv/x", "a b.txt", false).unwrap(),
            "lcd \"C:/Users/me/My Docs\"\ncd \"/srv/x\"\nput \"a b.txt\"\n"
        );
        assert_eq!(
            upload_script("C:/src", "/srv", "it's", true).unwrap(),
            "lcd \"C:/src\"\ncd \"/srv\"\n-mkdir \"it's\"\nput -r \"it's\"\n"
        );
    }

    #[test]
    fn builds_download_scripts() {
        assert_eq!(
            download_script("D:/dl", "/srv/x", "máy chủ", ".máy chủ.1.part").unwrap(),
            "lcd \"D:/dl\"\ncd \"/srv/x\"\nget -r \"máy chủ\" \".máy chủ.1.part\"\n"
        );
        let err = download_script("D:/dl", "/srv", "a\\b", "t").unwrap_err();
        assert_eq!(err.kind, ErrorKind::UnsupportedPath);
    }

    #[test]
    fn converts_local_paths_to_forward_slashes() {
        assert_eq!(
            local_arg(Path::new(r"C:\Users\me\máy chủ")).unwrap(),
            "C:/Users/me/máy chủ"
        );
    }

    #[test]
    fn rejects_names_windows_cannot_store() {
        for ok in ["a b.txt", "máy chủ", ".bashrc", "CONFIG", "COM10", "x.con"] {
            assert!(check_local_name(ok).is_ok(), "{ok}");
        }
        for bad in [
            "a:b", "what?", "trail.", "trail ", "con", "NUL.txt", "lpt1", "..", "",
        ] {
            let err = check_local_name(bad).unwrap_err();
            assert_eq!(err.kind, ErrorKind::UnsupportedPath, "{bad}");
        }
    }

    #[test]
    fn merges_folders_and_replaces_files_when_placing() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join("temp");
        let target = dir.path().join("target");
        fs::create_dir_all(temp.join("sub")).unwrap();
        fs::create_dir_all(target.join("sub")).unwrap();
        fs::write(temp.join("sub").join("new.txt"), "new").unwrap();
        fs::write(temp.join("same.txt"), "updated").unwrap();
        fs::write(target.join("same.txt"), "old").unwrap();
        fs::write(target.join("sub").join("kept.txt"), "kept").unwrap();

        place(&temp, &target).unwrap();
        assert!(!temp.exists());
        let read = |p: &[&str]| fs::read_to_string(p.iter().fold(target.clone(), |a, c| a.join(c)));
        assert_eq!(read(&["same.txt"]).unwrap(), "updated");
        assert_eq!(read(&["sub", "new.txt"]).unwrap(), "new");
        assert_eq!(read(&["sub", "kept.txt"]).unwrap(), "kept");
    }

    #[test]
    fn measures_local_files_and_folders() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("máy chủ");
        fs::create_dir_all(tree.join("sub dir")).unwrap();
        fs::write(tree.join("a.bin"), [0u8; 1000]).unwrap();
        fs::write(tree.join("sub dir").join("b"), "12345").unwrap();
        fs::create_dir(tree.join("empty")).unwrap();
        let size = |bytes, files| Size { bytes, files };
        assert_eq!(local_size(&tree.join("a.bin")), size(1000, None));
        assert_eq!(local_size(&tree), size(1005, Some(2)));
        assert_eq!(local_size(&tree.join("empty")), size(0, Some(0)));
        assert_eq!(local_size(&dir.path().join("missing")), size(0, None));
    }

    #[test]
    fn refuses_to_place_a_file_over_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join("temp");
        fs::write(&temp, "x").unwrap();
        fs::create_dir(dir.path().join("target")).unwrap();
        assert!(place(&temp, &dir.path().join("target")).is_err());
        assert!(temp.exists());
    }

    /// Byte-for-byte round trip through the real sftp.exe and a local
    /// sftp-server, with spaces and Unicode in names, plus conflict checks.
    #[cfg(windows)]
    #[test]
    fn round_trips_files_and_folders_through_the_system_sftp() {
        let remote = tempfile::tempdir().unwrap();
        let Some(connect) = sftp::testing::local_server(remote.path()) else {
            return;
        };
        let remote_dir = sftp::list(&connect, None, CHECK_TIMEOUT).unwrap().path;
        let local = tempfile::tempdir().unwrap();
        let file = local.path().join("máy chủ [1].bin");
        let bytes: Vec<u8> = (0..200_000u32).map(|i| (i * 7 % 251) as u8).collect();
        fs::write(&file, &bytes).unwrap();
        let folder = local.path().join("my folder");
        fs::create_dir_all(folder.join("sub dir")).unwrap();
        fs::write(folder.join("sub dir").join("ghi chú.txt"), "nested").unwrap();
        let cancel = AtomicBool::new(false);

        upload(&connect, &file, &remote_dir, false, &cancel).unwrap();
        upload(&connect, &folder, &remote_dir, false, &cancel).unwrap();
        assert_eq!(
            fs::read(remote.path().join("máy chủ [1].bin")).unwrap(),
            bytes
        );
        let nested = ["my folder", "sub dir", "ghi chú.txt"];
        let in_remote = nested
            .iter()
            .fold(remote.path().to_path_buf(), |a, c| a.join(c));
        assert_eq!(fs::read_to_string(in_remote).unwrap(), "nested");

        let err = upload(&connect, &file, &remote_dir, false, &cancel).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Exists, "{}", err.message);
        fs::write(&file, "changed").unwrap();
        upload(&connect, &file, &remote_dir, true, &cancel).unwrap();
        assert_eq!(
            fs::read(remote.path().join("máy chủ [1].bin")).unwrap(),
            b"changed"
        );

        let downloads = tempfile::tempdir().unwrap();
        for name in ["máy chủ [1].bin", "my folder"] {
            download(
                &connect,
                &remote_dir,
                name,
                downloads.path(),
                false,
                "t1",
                &cancel,
            )
            .unwrap();
        }
        assert_eq!(
            fs::read(downloads.path().join("máy chủ [1].bin")).unwrap(),
            b"changed"
        );
        let in_downloads = nested
            .iter()
            .fold(downloads.path().to_path_buf(), |a, c| a.join(c));
        assert_eq!(fs::read_to_string(&in_downloads).unwrap(), "nested");

        let err = download(
            &connect,
            &remote_dir,
            "my folder",
            downloads.path(),
            false,
            "t2",
            &cancel,
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Exists);
        fs::write(&in_downloads, "local edit").unwrap();
        download(
            &connect,
            &remote_dir,
            "my folder",
            downloads.path(),
            true,
            "t3",
            &cancel,
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&in_downloads).unwrap(), "nested");

        let names: Vec<_> = fs::read_dir(downloads.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 2, "no temporary copies left: {names:?}");
    }

    #[cfg(windows)]
    #[test]
    fn reports_missing_remote_files_and_cancellation() {
        let remote = tempfile::tempdir().unwrap();
        let Some(connect) = sftp::testing::local_server(remote.path()) else {
            return;
        };
        let remote_dir = sftp::list(&connect, None, CHECK_TIMEOUT).unwrap().path;
        let downloads = tempfile::tempdir().unwrap();

        let err = download(
            &connect,
            &remote_dir,
            "missing.txt",
            downloads.path(),
            false,
            "t1",
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Remote, "{}", err.message);

        fs::write(remote.path().join("a.txt"), "a").unwrap();
        let err = download(
            &connect,
            &remote_dir,
            "a.txt",
            downloads.path(),
            false,
            "t2",
            &AtomicBool::new(true),
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Cancelled);
        assert_eq!(fs::read_dir(downloads.path()).unwrap().count(), 0);
    }

    /// Same round trip against a real server through the system ssh config:
    /// `$env:SIMTER_TEST_ALIAS="host"; cargo test -- --ignored real_host`.
    /// Works only inside a fresh `mktemp -d /tmp/simter-test.XXXXXX` folder,
    /// which it removes afterwards. Needs key or agent authentication and a
    /// known host key.
    #[cfg(windows)]
    #[test]
    #[ignore = "needs SIMTER_TEST_ALIAS and a reachable SSH server"]
    fn real_host_round_trip() {
        let alias = std::env::var("SIMTER_TEST_ALIAS").expect("set SIMTER_TEST_ALIAS");
        let ssh = crate::ssh_bin::locate("ssh.exe").unwrap();
        let sftp_exe = crate::ssh_bin::locate("sftp.exe").unwrap();
        let remote = |cmd: &str| {
            let out = std::process::Command::new(&ssh)
                .args(["-o", "BatchMode=yes", "--", &alias, cmd])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{cmd}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout).unwrap()
        };
        let remote_dir = remote("mktemp -d /tmp/simter-test.XXXXXX")
            .trim()
            .to_owned();
        assert!(remote_dir.starts_with("/tmp/simter-test."), "{remote_dir}");
        // Runs even if an assertion fails; must not panic itself.
        struct Cleanup<F: Fn()>(F);
        impl<F: Fn()> Drop for Cleanup<F> {
            fn drop(&mut self) {
                (self.0)();
            }
        }
        let _cleanup = Cleanup(|| {
            let status = std::process::Command::new(&ssh)
                .args(["-o", "BatchMode=yes", "--", &alias])
                .arg(format!("rm -rf -- '{remote_dir}'"))
                .status();
            if !matches!(status, Ok(s) if s.success()) {
                eprintln!("could not remove {remote_dir} on {alias}: {status:?}");
            }
        });
        let connect = || sftp::sftp_command(&sftp_exe, None, &alias, None);

        let local = tempfile::tempdir().unwrap();
        let file = local.path().join("máy chủ [1].bin");
        let bytes: Vec<u8> = (0..1_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
        fs::write(&file, &bytes).unwrap();
        let folder = local.path().join("my folder");
        fs::create_dir_all(folder.join("sub dir")).unwrap();
        fs::write(folder.join("sub dir").join("ghi chú.txt"), "nested").unwrap();
        let cancel = AtomicBool::new(false);

        upload(&connect, &file, &remote_dir, false, &cancel).unwrap();
        upload(&connect, &folder, &remote_dir, false, &cancel).unwrap();
        let listing = sftp::list(&connect, Some(&remote_dir), CHECK_TIMEOUT).unwrap();
        assert!(listing.unparsed.is_empty(), "{:?}", listing.unparsed);
        let kinds: HashMap<_, _> = listing
            .entries
            .iter()
            .map(|e| (e.name.as_str(), (e.kind, e.size)))
            .collect();
        assert_eq!(
            kinds["máy chủ [1].bin"],
            (sftp::Kind::File, bytes.len() as u64)
        );
        assert_eq!(kinds["my folder"].0, sftp::Kind::Dir);
        let content = remote(&format!(
            "cat -- '{remote_dir}/my folder/sub dir/ghi chú.txt'"
        ));
        assert_eq!(content, "nested");

        let err = upload(&connect, &file, &remote_dir, false, &cancel).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Exists, "{}", err.message);

        let downloads = tempfile::tempdir().unwrap();
        for name in ["máy chủ [1].bin", "my folder"] {
            download(
                &connect,
                &remote_dir,
                name,
                downloads.path(),
                false,
                "t1",
                &cancel,
            )
            .unwrap();
        }
        assert_eq!(
            fs::read(downloads.path().join("máy chủ [1].bin")).unwrap(),
            bytes
        );
        let nested = downloads
            .path()
            .join("my folder")
            .join("sub dir")
            .join("ghi chú.txt");
        assert_eq!(fs::read_to_string(nested).unwrap(), "nested");
        let err = download(
            &connect,
            &remote_dir,
            "missing.txt",
            downloads.path(),
            false,
            "t2",
            &cancel,
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Remote, "{}", err.message);
        assert_eq!(fs::read_dir(downloads.path()).unwrap().count(), 2);
    }
}
