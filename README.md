# Simter

Simter (Simple Terminal) is a lightweight Windows desktop app: searchable SSH host list, embedded SSH terminal, and an
adjacent remote file browser in one window. See [spec.md](spec.md) for full requirements.

> Status: early development. Nothing is runnable yet.

## Architecture

| Concern | Implementation |
|---|---|
| App shell | [Tauri 2](https://tauri.app) (system WebView2, small binary) |
| Terminal rendering | [xterm.js](https://xtermjs.org) |
| PTY | [portable-pty](https://crates.io/crates/portable-pty) (Windows ConPTY) running the system `ssh.exe <alias>` |
| Host discovery | Reads `Host`/`Include` from the SSH config; effective options via `ssh -G <alias>` |
| File transfer | System `sftp.exe <alias>` subprocess, so all OpenSSH config (ProxyJump, IdentityFile, ports) applies |
| Frontend | TypeScript + Vite, no UI framework |
| File pickers | [tauri-plugin-dialog](https://v2.tauri.app/plugin/dialog/) (native open dialogs; only `dialog:allow-open` is granted) |

OpenSSH remains the only connection/authentication implementation. The app never rewrites
the SSH config to read it. When ssh (the terminal's or the file pane's) needs a password, passphrase
or host-key confirmation, ssh asks the app through `SSH_ASKPASS` (see below). The app writes no passwords
or keys to its own files; a password is kept only if you tick **Save password**, and then in
Windows Credential Manager. Persistent shell sessions
are Byobu/tmux's job on the remote host.

## Prerequisites (Windows 10/11)

- Windows OpenSSH client (`C:\Windows\System32\OpenSSH\ssh.exe`, `sftp.exe`)
- Microsoft Edge WebView2 runtime (preinstalled on Windows 11)

To build from source, additionally:

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install Rustlang.Rustup
winget install OpenJS.NodeJS.LTS
```

## Build and test

```powershell
npm install
npm run tauri dev        # run the app with hot reload
npm run tauri build      # release build + NSIS installer under src-tauri/target/release/bundle
```

Checks:

```powershell
npm test                 # frontend unit tests (Vitest)
npm run build            # TypeScript type-check + Vite build
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Optional tests against a real server (key or agent auth, host key already known). The SFTP
round trip works only inside a fresh `/tmp/simter-test.*` folder on the server and removes it
after; the askpass test runs the built app as ssh's helper with a temporary empty
known_hosts, so ssh asks to accept the host key (run `cargo build` first):

```powershell
$env:SIMTER_TEST_ALIAS = "my-test-host"
cargo build
cargo test real_host -- --ignored
```

Currently the app lists and searches hosts, and Connect opens `ssh <alias>` in an embedded
terminal tab with a remote file browser beside it that uploads and downloads files and folders.

Automated tests use fakes and a local SFTP server; they do not prove real SSH, Byobu or ConPTY
behavior. Acceptance needs a manual run on Windows 10/11 against a Linux server, recorded with
[docs/TESTBED.md](docs/TESTBED.md). No such run has been recorded yet: **NOT TESTED ON TESTBED**.

## Basic operation

- Hosts you opened recently are listed first, most recent first, above a separator line; the
  rest keep their order in the config. The app remembers the last 50 aliases opened (by alias
  name, for any config file). Within a search, recent hosts come first in each match rank.
- Type in the search box to filter aliases (case-insensitive; prefix, then substring, then
  in-order letter matches).
- Up/Down move the selection, Enter or double-click connects, Escape clears the search.
- The selected host shows its effective `user@hostname:port`, resolved in the background with
  `ssh -G <alias>` (5 s timeout). "details unavailable" means `ssh -G` failed; hover for why.
- **Open config** opens the SSH config in VS Code at the selected host's `Host` line
  (`code -g`, if `code` is on PATH), otherwise in Notepad. The app never writes the config;
  save in the editor, then press **Refresh**.
- Hosts and Files share the side bar on the right. Click a view's header to collapse or expand
  it; drag the divider between them (or focus it and use Up/Down) to share the height. The
  panel button at the far right hides or shows the whole side bar; drag the divider left of it
  (or focus it and use Left/Right) to resize. There is no keyboard shortcut, so every key stays
  with the terminal (Ctrl+B is the tmux prefix). The layout is saved.
- Click the config path at the bottom of the Hosts view to use a different SSH config file. It is
  passed to `ssh`, `ssh -G` and `sftp` as `-F` (which also makes ssh skip the system-wide
  config). Pick the default file again to go back. Open tabs keep the config they were opened
  with.

## Preferences

Stored in `%APPDATA%\dev.simter.desktop\prefs.json`, separate from the SSH config. The
app saves the chosen config file (`configPath`), the side bar layout (`layout`: width, Files height, hidden/collapsed views) and the
recently opened host aliases (`recentHosts`; delete the key to clear the list). On first
start Simter copies the settings of its earlier name, SSH Workspace
(`dev.sshworkspace.desktop`), if it has none of its own yet. No
passwords, keys or host settings are stored there (saved passwords live in Windows Credential
Manager, see below). To use another editor for **Open config**, add it by
hand, for example:

```json
{ "editor": "C:\\Program Files\\Notepad++\\notepad++.exe" }
```

It is run as `editor <file>` (no line number). Unknown keys are kept. If the file is not valid
JSON the app says so, uses defaults, and does not overwrite it.
- **Refresh** re-reads the SSH config after external edits.
- Config problems are listed above the host list. Click the count to expand them.
- In the terminal, all keys (including F1–F12) go to the remote side. The browser's own
  shortcuts (F5 reload, F3/Ctrl+F find, F7 caret browsing, F12 devtools) are disabled.
- Each host opens in its own tab; connecting to a host that already has a tab switches to it.
  The dot shows the state: amber connecting, green connected, grey ended, red failed (exit 255
  means ssh itself failed, e.g. unreachable host or authentication).
- **Reconnect** runs `ssh <alias>` again in the same tab, keeping its scrollback. It starts a new
  login shell; your Byobu/tmux session reattaches only if the login does that (e.g.
  `byobu-enable`), otherwise run `byobu` yourself.
- The Files view shows the active tab's host and lists the remote login folder over SFTP. Double-click or
  Enter opens a folder (or a link to one), Backspace or ↑ goes up, and you can type a path
  into the path box.
- **Upload files…** / **Upload folder…** copy into the folder shown in the pane; **Download…**
  copies the selected file or folder into a local folder you pick. Transfers run one at a time
  and are listed below the pane with running/done/skipped/cancelled/failed state.
- If ssh needs a password or key passphrase, a dialog asks for it, for the terminal and the
  file pane alike. The tab remembers it in memory for its terminal reconnects, listings and
  transfers, so it is asked once per tab, and forgets it when the tab
  closes. If the server rejects it you are asked again; Cancel stops that connection. A new
  host key is shown with its fingerprint and is accepted only if you click **Accept**.
- Tick **Save password in Windows Credential Manager** in that dialog to keep the password
  after the tab closes (off by default). It is saved as a generic Windows credential named
  `simter:<alias>:<prompt>`, encrypted for your Windows user and not roamed to other
  PCs, and used the next time that host asks the same prompt. If the server rejects a saved
  password, the app deletes it and asks again. To remove one yourself, open Control Panel >
  Credential Manager > Windows Credentials and remove the `simter:` entry. Passwords saved
under the app's earlier name (`ssh-workspace:...`) are moved to `simter:` when first used.
  This is a deliberate, opt-in exception to the spec's "no password store" rule, made at the
  user's request; the app uses the OS store rather than keeping its own.
  How it works: ssh runs with `SSH_ASKPASS` set to the app's own executable (sftp also gets
  `-o BatchMode=no`), which passes the prompt to the running app over a local named pipe.
- If the destination already exists you choose Overwrite, Skip, or Cancel (which also stops the
  rest of that batch). Nothing is overwritten without asking. Overwriting a folder merges into it.
- Closing a tab (×) ends only that tab's ssh process; Byobu sessions keep running on the host.
- Copy: Ctrl+Shift+C, or Ctrl+C while text is selected (without a selection Ctrl+C is the
  interrupt). Paste: Ctrl+V or Ctrl+Shift+V.

## Supported SSH config behavior

Host discovery reads `%USERPROFILE%\.ssh\config`, or the chosen file, (read-only) and lists aliases from `Host` lines:

- Concrete aliases only. Patterns containing `*` or `?`, and negated `!patterns`, are not listed.
- `Include` is followed with glob wildcards, `~`, `${ENV}`, `%d` and `%%`. Relative paths
  resolve under `%USERPROFILE%\.ssh`. Nesting depth is limited to 16, and cycles are reported.
- An alias declared in more than one place has no single source location, so "Open config"
  falls back to the main file.
- Hosts found in a file included from inside a `Host`/`Match` block are marked conditional,
  because ssh only reads that file when the block matches.
- Problems (missing includes, unsupported `%` tokens, unterminated quotes, unreadable files)
  are reported per file and line. Hosts found elsewhere are still listed.

Discovery only decides which aliases to show. Effective settings come from `ssh -G <alias>`,
and connections always use `ssh <alias>`.

## Known limitations

- Each listing and transfer opens a new SFTP connection, which is slower through jump hosts.
- Password and host-key prompts appear in a dialog, not in the terminal. Prompts the remote
  shell prints after login (such as `sudo`) still appear in the terminal.
- Remote paths containing a backslash, a line break, or both `'` and `"` cannot be opened:
  Windows `sftp` rewrites backslashes in batch commands and has no other escape.
- No byte-level progress: `sftp` prints no progress meter without a terminal, so transfers show
  only their state.
- Downloads go to a hidden `.name.<id>.part` copy in the target folder and are moved into place
  when complete, so a cancelled or failed download leaves nothing behind. A cancelled **upload**
  can leave a partial file or folder on the server.
- Local file names that Windows cannot store (`<>:"|?*`, trailing dot or space, `CON`, `NUL`…)
  are refused before downloading.
- Symlinks are shown with their own icon. Opening one tries it as a folder and reports an
  error if it is not one.
