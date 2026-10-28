//! Opens remote files for viewing: a read-only copy, in a folder of its own
//! under the temp folder, opened with the app Windows associates with its
//! type. Changes to the copy are never uploaded back.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::sftp::{Error, ErrorKind};

/// Where copies opened from the file pane are kept.
pub fn temp_root() -> PathBuf {
    std::env::temp_dir().join("Simter")
}

/// Creates the empty folder for one copy. `tag` keeps copies apart, so the
/// file keeps its own name (which its app goes by) and a copy still open from
/// an earlier Open is never replaced.
pub fn copy_dir(root: &Path, tag: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(root)?;
    let dir = root.join(tag);
    fs::create_dir(&dir)?;
    Ok(dir)
}

/// Marks the copy read-only, so an editor warns before saving changes that
/// would not reach the host, then opens it.
pub fn view(file: &Path) -> Result<(), Error> {
    make_read_only(file).map_err(|e| {
        Error::new(
            ErrorKind::Local,
            format!("cannot mark {} read-only: {e}", file.display()),
        )
    })?;
    open(file)
}

fn make_read_only(file: &Path) -> io::Result<()> {
    let mut permissions = fs::metadata(file)?.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(file, permissions)
}

/// Interpreters that installers commonly set to run such files when opened
/// (Git Bash, Python, Perl, Ruby, PowerShell, Java...), which Windows does
/// not count as dangerous.
const SCRIPTS: [&str; 17] = [
    "sh", "bash", "zsh", "ksh", "csh", "fish", "py", "pyw", "pyz", "pl", "rb", "php", "ps1",
    "psm1", "lua", "tcl", "jar",
];

/// Whether opening a file with extension `ext` (without the dot) may run it
/// rather than show it: a type Windows deems `dangerous` (programs,
/// shortcuts, Windows scripts), one listed in `pathext`, or a common script.
pub fn runs_code(ext: &str, pathext: Option<&str>, dangerous: impl Fn(&str) -> bool) -> bool {
    let ext = ext.to_ascii_lowercase();
    let in_pathext = pathext.is_some_and(|list| {
        list.split(';')
            .any(|e| e.trim().trim_start_matches('.').eq_ignore_ascii_case(&ext))
    });
    SCRIPTS.contains(&ext.as_str()) || in_pathext || dangerous(&format!(".{ext}"))
}

/// Opens `file` with its app. A file that would run, or whose type has no
/// app, opens with the app the user picks in Windows' "Open with" dialog;
/// closing that dialog is a cancellation.
#[cfg(windows)]
fn open(file: &Path) -> Result<(), Error> {
    let file = file.to_path_buf();
    // COM for shell extensions, on a thread of its own so its apartment does
    // not stick to a thread of the async runtime's pool.
    std::thread::spawn(move || shell::open(&file))
        .join()
        .unwrap_or_else(|_| Err(Error::new(ErrorKind::Other, "opening the file panicked")))
}

#[cfg(not(windows))]
fn open(file: &Path) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        format!("cannot open {} outside Windows", file.display()),
    ))
}

#[cfg(windows)]
mod shell {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::core::{w, HRESULT, PCWSTR};
    use windows::Win32::Foundation::ERROR_CANCELLED;
    use windows::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::UI::Shell::{
        AssocIsDangerous, SHOpenWithDialog, ShellExecuteW, OAIF_ALLOW_REGISTRATION, OAIF_EXEC,
        OAIF_REGISTER_EXT, OPENASINFO, OPEN_AS_INFO_FLAGS, SE_ERR_NOASSOC,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    use super::{runs_code, Error, ErrorKind};

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain([0]).collect()
    }

    pub fn dangerous(ext: &str) -> bool {
        let ext = wide(ext.as_ref());
        // SAFETY: `ext` is NUL-terminated and outlives the call.
        unsafe { AssocIsDangerous(PCWSTR(ext.as_ptr())).as_bool() }
    }

    pub fn open(file: &Path) -> Result<(), Error> {
        let path = wide(file.as_os_str());
        let path = PCWSTR(path.as_ptr());
        let runs = file
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| runs_code(ext, std::env::var("PATHEXT").ok().as_deref(), dangerous));
        // SAFETY: `path` points into a NUL-terminated buffer that outlives
        // the calls; COM is initialized on this thread only, and released
        // when that succeeded.
        unsafe {
            let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
            let result = if runs {
                // Never registers the choice, so the next one asks again.
                open_with(path, OAIF_EXEC, file)
            } else {
                let code =
                    ShellExecuteW(None, w!("open"), path, None, None, SW_SHOWNORMAL).0 as isize;
                match code {
                    33.. => Ok(()),
                    c if c == SE_ERR_NOASSOC as isize => open_with(
                        path,
                        OAIF_ALLOW_REGISTRATION | OAIF_REGISTER_EXT | OAIF_EXEC,
                        file,
                    ),
                    c => Err(Error::new(
                        ErrorKind::Local,
                        format!("Windows could not open {} (error {c})", file.display()),
                    )),
                }
            };
            if init.is_ok() {
                CoUninitialize();
            }
            result
        }
    }

    /// # Safety
    /// `path` must be a valid NUL-terminated path, with COM initialized.
    unsafe fn open_with(path: PCWSTR, flags: OPEN_AS_INFO_FLAGS, file: &Path) -> Result<(), Error> {
        let info = OPENASINFO {
            pcszFile: path,
            pcszClass: PCWSTR::null(),
            oaifInFlags: flags,
        };
        SHOpenWithDialog(None, &info).map_err(|e| {
            if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                Error::new(
                    ErrorKind::Cancelled,
                    format!("No app was chosen; the copy is {}", file.display()),
                )
            } else {
                Error::new(
                    ErrorKind::Local,
                    format!("cannot open {}: {}", file.display(), e.message()),
                )
            }
        })
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn asks_windows_which_types_are_dangerous() {
            assert!(super::dangerous(".exe"));
            assert!(super::dangerous(".bat"));
            assert!(!super::dangerous(".txt"));
            assert!(!super::dangerous(".log"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_one_empty_folder_per_copy() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Simter");
        let first = copy_dir(&root, "1-1").unwrap();
        assert_eq!(first, root.join("1-1"));
        assert_eq!(fs::read_dir(&first).unwrap().count(), 0);
        copy_dir(&root, "1-2").unwrap();
        let again = copy_dir(&root, "1-1").unwrap_err();
        assert_eq!(again.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn marks_copies_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ghi chú.txt");
        fs::write(&file, "x").unwrap();
        make_read_only(&file).unwrap();
        assert!(fs::metadata(&file).unwrap().permissions().readonly());
        assert!(fs::write(&file, "changed").is_err());
        // Read-only files must not stop the temp folder from being removed.
        drop(dir);
        assert!(!file.exists());
    }

    #[test]
    fn tells_files_that_would_run_from_files_to_show() {
        let dangerous = |ext: &str| [".exe", ".bat"].contains(&ext);
        let pathext = Some(".COM;.EXE;.BAT;.CMD;.VBS;.JS;.PY");
        for ext in ["exe", "BAT", "cmd", "js", "py", "sh", "ps1", "jar"] {
            assert!(runs_code(ext, pathext, dangerous), "{ext}");
        }
        for ext in ["txt", "log", "json", "c", "md", "pdf", "png", "conf"] {
            assert!(!runs_code(ext, pathext, dangerous), "{ext}");
        }
        assert!(runs_code("cmd", Some("  .cmd ;"), |_| false));
        assert!(!runs_code("cmd", None, |_| false));
    }
}
