# Simter

Simter (Simple Terminal) is a lightweight Windows app for working on SSH hosts: a searchable
host list from your SSH config, an embedded terminal per host, and an SFTP file pane in one
window. It runs the Windows OpenSSH client (`ssh.exe`, `sftp.exe`) with your existing config,
so keys, agents, `ProxyJump` and `Include` work as they do in a console. The file pane keeps one
`ssh -s <alias> sftp` process per tab for browsing and speaks SFTP over it with
[russh-sftp](https://crates.io/crates/russh-sftp), which has no SSH or crypto code of its own. See [spec.md](spec.md)
for the full requirements.

## Install

Requirements: Windows 10 or 11 with the OpenSSH client (built in; check with `ssh -V`) and the
WebView2 runtime (preinstalled on Windows 11).

1. Build the installer (see [Build from source](#build-from-source)), or take
   `Simter_<version>_x64-setup.exe` from someone who built it.
2. Run it. It installs for the current user only, without administrator rights, and adds Simter
   to the Start menu. The installer is not code-signed, so Windows SmartScreen may warn: choose
   **More info → Run anyway**.
3. Uninstall from **Settings → Apps → Installed apps → Simter**.

Simter reads `%USERPROFILE%\.ssh\config` and never writes to it. Add hosts there as usual.

## Use

- **Hosts** (right side bar): type to filter, Up/Down to select, Enter or double-click to
  connect. Recently opened hosts are listed first. The selected host shows its effective
  `user@hostname:port` from `ssh -G`.
- **Open config** opens the SSH config at that host's line in VS Code (or Notepad); press
  **Refresh** after saving. Click the config path at the bottom to use another config file.
- **Tabs**: one per host. The dot shows amber connecting, green connected, grey ended, red
  failed. **Reconnect** runs `ssh` again in the same tab. Closing a tab ends only that ssh
  process; Byobu/tmux sessions keep running on the host.
- **Terminal**: every key, including F1–F12 and Ctrl+B, goes to the remote side. Copy with
  Ctrl+Shift+C (or Ctrl+C while text is selected); paste with Ctrl+V or Ctrl+Shift+V.
- **Files** (right side bar): the active tab's host over SFTP. Double-click or Enter opens a
  folder, Backspace goes up, or type a path. Right-click (or press the context-menu key or
  Shift+F10): on empty space for **Upload files…** and **Upload folder…**, on an item for
  **Download…** and **Delete…**. Transfers run one at a time; an existing target asks Overwrite,
  Skip or Cancel. Delete asks first and cannot be undone: a folder goes with everything in it,
  and a link is deleted without touching what it points to.
- **Layout**: click a view's header to collapse it, drag the dividers to resize, and use the
  panel button at the far right to hide the side bar. The layout is saved.
- **Passwords**: password, passphrase and new host-key prompts appear in a dialog, once per tab
  for both terminal and files. A host key is accepted only when you click **Accept**. Tick
  **Save password in Windows Credential Manager** to keep a password; it is stored as
  `simter:<alias>:<prompt>` under Control Panel → Credential Manager → Windows Credentials,
  removed there, and deleted automatically if the server rejects it.

Settings (config file, recent hosts, layout) are in `%APPDATA%\dev.simter.desktop\prefs.json`.
To use another editor for **Open config**, add `"editor": "C:\\path\\to\\editor.exe"` there.
No passwords or keys are stored in it.

## Limitations

- Each transfer opens its own SFTP connection, which is slower through jump hosts. Browsing
  reuses one connection per tab.
- Transfers show their state but not byte progress. A cancelled upload may leave a partial file.
- Remote paths containing a backslash, a line break, or both `'` and `"` cannot be opened.
- Wildcard `Host` patterns are not listed; only concrete aliases are.
- Not yet validated end to end on a Windows + Linux testbed ([docs/TESTBED.md](docs/TESTBED.md)).

## Build from source

Prerequisites:

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install Rustlang.Rustup
winget install OpenJS.NodeJS.LTS
```

Build and run:

```powershell
npm install
npm run tauri dev     # run with hot reload
npm run tauri build   # installer in src-tauri\target\release\bundle\nsis\
```

Checks:

```powershell
npm test
npm run build
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Optional tests against a real server that logs in with a key (they work only in a fresh
`/tmp/simter-test.*` folder there and remove it after):

```powershell
$env:SIMTER_TEST_ALIAS = "my-test-host"
cargo build
cargo test real_host -- --ignored
```

Stack: Tauri 2, xterm.js, portable-pty (ConPTY), TypeScript + Vite. Password prompts reach the
app through `SSH_ASKPASS`, pointed at Simter's own executable, over a local named pipe.
