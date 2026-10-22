# Implementation plan

Delivery plan for the MVP in [spec.md](../spec.md). Work proceeds in small, independently
verified slices per [AGENTS.md](../AGENTS.md). Update the status column as slices land.

## Layout

```
src-tauri/src/
  ssh_bin.rs      locate the real ssh.exe / sftp.exe
  hosts/          Host/Include discovery + `ssh -G` resolution
  terminal.rs     ConPTY session: spawn `ssh <alias>`, I/O, resize, exit
  sftp/           list, upload, download, conflict checks, error types
  sftp_session.rs long-lived per-tab SFTP connection for browsing
  config_open.rs  open config in an editor at a line
  prefs.rs        app-only preferences JSON in %APPDATA%
src/              TypeScript UI: sidebar, tabs, splitter, xterm.js, file pane
tests/fixtures/   SSH config fixtures
docs/             design notes, testbed report template
```

## Slices

| # | Slice | Delivers | Verification | Status |
|---|---|---|---|---|
| 1 | Repository setup | Project-local repo, README, .gitignore | — | done |
| 2 | Host discovery (Rust) | Lexical `Host`/`Include` scan (globs, `~`, relative paths, depth limit), skip wildcard/negated patterns, record source file+line, per-file errors without dropping working hosts | Unit tests: nested Include, Match blocks, quoting, missing files, include cycles | done |
| 3 | Tauri skeleton + host sidebar | Window, searchable list, arrow keys + Enter, Refresh, error banner | `cargo test`, frontend filter tests, app launches | done |
| 4 | Terminal spike (highest risk) | xterm.js + portable-pty running `ssh.exe <alias>` in one tab; confirm WebView2 does not consume F3/F5/F7/F12 or Ctrl shortcuts (disable browser accelerator keys) | Manual check with real `ssh` and Byobu F2–F9; revisit stack if it fails | done (testbed pending) |
| 5 | Tabs + connection state | Multiple tabs, connected/disconnected/exit state, Reconnect re-runs `ssh <alias>`, closing one tab leaves others intact | Unit tests: argv construction (no shell strings), exit classification | done (testbed pending) |
| 6 | `ssh -G` details | Lazy background resolution of user/hostname/port with timeout | Parser tests, timeout and failure cases | done |
| 7 | File pane: browse | Resizable splitter beside the terminal; listing, parent, refresh via `sftp -b - <alias>` per operation | Listing parser tests (spaces, Unicode, symlinks), error classification | done (testbed pending) |
| 8 | File pane: transfers | Upload/download files and folders; Overwrite/Skip/Cancel on conflict; download to temp then rename; cancel; running/done/failed state | Integration tests with checksums against a test server | done (testbed pending) |
| 9 | Config editing (W3) | Open config at the host's line (VS Code `-g`, else Notepad, or configured editor); refresh; custom config path passed as `-F` to ssh and sftp | Tests asserting config files are never written | done (testbed pending) |
| 10 | Preferences + polish | Prefs JSON (config path, editor, splitter), README limitations, testbed report template. Recent hosts dropped: not needed by any acceptance criterion | Full suite | done (testbed pending) |
| 11 | Testbed acceptance | Spec §7 criteria 1–9 on Windows + Linux with Byobu | Written report; until then `NOT TESTED ON TESTBED` | |

## Technical decisions

- **ssh.exe resolution:** prefer `%WINDIR%\System32\OpenSSH\`, then PATH, with a preference
  override. Git for Windows ships its own `ssh` that can shadow the system client on PATH.
- **No shell interpolation:** the alias is passed as a discrete argv element (FR-2).
- **Remote listing:** parse `sftp` `ls -la` output. The listing is formatted client-side by
  OpenSSH's sftp, so the layout is deterministic, and it works on SFTP-only servers.
- **Transfers: one sftp process per operation**: simple, failure-isolated, cancel = kill.
  Cost: connection setup per transfer (slower through jump hosts).
- **Browsing: one long-lived connection per tab** (`sftp_session.rs`), so changing folders does
  not reconnect. Windows `sftp.exe` buffers its output when piped, so it cannot be driven
  interactively; instead the app runs `ssh -s <alias> sftp` (the same arguments sftp.exe
  passes, including `RemoteCommand=none`) and the russh-sftp crate speaks SFTP over its
  stdin/stdout. OpenSSH still does config, authentication and host keys. A request that
  times out or a dead ssh process drops the connection; the next request reconnects. Closing
  the tab kills the process.
- **Host keys:** the terminal handles prompts normally. The file pane uses `BatchMode=yes`,
  so an unknown host fails with guidance to accept the key in the terminal; never auto-accepted.
- **Progress:** sftp prints no progress meter without a TTY, so progress is measured from the
  files: the total from the source (local walk, or a remote walk over the tab's SFTP
  connection), the bytes done from the copy every 500 ms (the local `.part` copy for
  downloads, the remote target for uploads). The bar stays indeterminate until the total is
  known or if the connection cannot measure; the transfer itself never depends on it.

## Decisions (defaults taken; change if needed)

1. **Password-only hosts in the file pane:** not supported in the MVP. Windows OpenSSH has no
   ControlMaster, and `BatchMode=yes` needs key or agent auth. The pane shows a clear error,
   and README documents the limitation. Deviation from W2/FR-4 is flagged explicitly.
2. **Test servers:** to be confirmed. Integration tests will target a disposable Linux sshd
   (Docker if available) and be skipped with a stated reason when absent.
3. **Default editor:** VS Code (`code -g file:line`) if on PATH, otherwise Notepad.

## Risks

- WebView2 consuming function keys or Ctrl shortcuts (addressed first, in slice 4).
- Fragility of parsing sftp text output; mitigated by fixture tests per OpenSSH version.
