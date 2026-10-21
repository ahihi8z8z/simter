//! Opens an SSH config file in an external editor.
//!
//! The app never writes config files itself; edits happen in the editor and
//! the host list is refreshed afterwards.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Builds the command that opens `file`: the configured `editor` as
/// `editor <file>` if set, else VS Code at `line` (`code.cmd -g file:line`) if
/// it is on PATH, else Notepad, which opens at the top.
pub fn editor_command(
    file: &Path,
    line: Option<usize>,
    editor: Option<&Path>,
) -> Result<Command, String> {
    editor_command_in(
        file,
        line,
        editor,
        std::env::var_os("WINDIR"),
        std::env::var_os("PATH"),
    )
}

fn editor_command_in(
    file: &Path,
    line: Option<usize>,
    editor: Option<&Path>,
    windir: Option<OsString>,
    path: Option<OsString>,
) -> Result<Command, String> {
    if !file.is_absolute() || !file.is_file() {
        return Err(format!("{} is not a file", file.display()));
    }
    if let Some(editor) = editor {
        let mut cmd = Command::new(editor);
        cmd.arg(file);
        return Ok(cmd);
    }
    let code = path
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join("code.cmd"))
        .find(|candidate| candidate.is_file());
    if let Some(code) = code {
        // std escapes arguments for .cmd files, and refuses ones it cannot
        // pass safely, so the path never reaches cmd.exe unquoted.
        let mut target = file.as_os_str().to_owned();
        if let Some(line) = line {
            target.push(format!(":{line}"));
        }
        let mut cmd = Command::new(code);
        cmd.arg("-g").arg(target);
        return Ok(cmd);
    }
    let notepad = windir
        .map(|w| PathBuf::from(w).join("System32").join("notepad.exe"))
        .filter(|p| p.is_file())
        .ok_or("Neither VS Code (code.cmd on PATH) nor Notepad was found")?;
    let mut cmd = Command::new(notepad);
    cmd.arg(file);
    Ok(cmd)
}

/// Starts the editor without waiting for it to exit.
pub fn open(file: &Path, line: Option<usize>, editor: Option<&Path>) -> Result<(), String> {
    let mut cmd = editor_command(file, line, editor)?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // code.cmd runs through cmd.exe; without this a console flashes up.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map(drop)
        .map_err(|e| format!("could not start {:?}: {e}", cmd.get_program()))
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

    fn args(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    struct Fixture {
        _dir: TempDir,
        config: PathBuf,
        windir: PathBuf,
        bin: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("my ssh").join("config");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "Host a\n  HostName a.example\n").unwrap();
        let windir = dir.path().join("Windows");
        touch(&windir.join("System32").join("notepad.exe"));
        let bin = dir.path().join("VS Code").join("bin");
        fs::create_dir_all(&bin).unwrap();
        Fixture {
            _dir: dir,
            config,
            windir,
            bin,
        }
    }

    #[test]
    fn prefers_vs_code_at_the_line() {
        let f = fixture();
        touch(&f.bin.join("code.cmd"));
        let path = std::env::join_paths([&f.bin]).unwrap();
        let cmd = editor_command_in(&f.config, Some(12), None, Some(f.windir.into()), Some(path))
            .unwrap();
        assert_eq!(cmd.get_program(), f.bin.join("code.cmd"));
        assert_eq!(args(&cmd), ["-g", &format!("{}:12", f.config.display())]);
    }

    #[test]
    fn falls_back_to_notepad_without_line() {
        let f = fixture();
        let path = std::env::join_paths([&f.bin]).unwrap();
        let cmd = editor_command_in(
            &f.config,
            Some(12),
            None,
            Some(f.windir.clone().into()),
            Some(path),
        )
        .unwrap();
        assert_eq!(
            cmd.get_program(),
            f.windir.join("System32").join("notepad.exe")
        );
        assert_eq!(args(&cmd), [f.config.display().to_string()]);
    }

    #[test]
    fn configured_editor_wins() {
        let f = fixture();
        touch(&f.bin.join("code.cmd"));
        let path = std::env::join_paths([&f.bin]).unwrap();
        let editor = Path::new(r"C:\Tools\my editor.exe");
        let cmd = editor_command_in(
            &f.config,
            Some(3),
            Some(editor),
            Some(f.windir.into()),
            Some(path),
        )
        .unwrap();
        assert_eq!(cmd.get_program(), editor);
        assert_eq!(args(&cmd), [f.config.display().to_string()]);
    }

    #[test]
    fn rejects_missing_and_relative_files() {
        let f = fixture();
        let windir = Some(f.windir.clone().into_os_string());
        let missing = f.config.with_file_name("nope");
        assert!(editor_command_in(&missing, None, None, windir.clone(), None).is_err());
        assert!(editor_command_in(Path::new("config"), None, None, windir, None).is_err());
    }

    #[test]
    fn building_the_command_leaves_the_config_untouched() {
        let f = fixture();
        let before = fs::metadata(&f.config).unwrap().modified().unwrap();
        editor_command_in(&f.config, Some(1), None, Some(f.windir.into()), None).unwrap();
        assert_eq!(
            fs::read_to_string(&f.config).unwrap(),
            "Host a\n  HostName a.example\n"
        );
        assert_eq!(fs::metadata(&f.config).unwrap().modified().unwrap(), before);
    }
}
