# Testbed acceptance report

Copy this file to `docs/testbed/YYYY-MM-DD.md`, fill it in, and commit it. Until a filled-in
report shows all criteria passing, the build is **NOT TESTED ON TESTBED** and is not accepted
against `spec.md` §7.

Use disposable test identities and non-production hosts. Do not paste private keys, passwords
or real hostnames you do not want in the repository; use placeholders.

## Environment

| Item | Value |
|---|---|
| Date / tester | |
| App commit (`git rev-parse --short HEAD`) | |
| Build | `npm run tauri build` installer / `npm run tauri dev` |
| Windows edition and build (`winver`) | |
| OpenSSH client (`ssh -V`) | |
| WebView2 runtime version | |
| Linux server distro, `sshd -V` / OpenSSH version | |
| Byobu version and backend (tmux/screen) | |
| Jump host (ProxyJump) used | yes / no |
| Authentication used | key file / ssh-agent / password |

### SSH config under test

Describe (or attach a redacted copy of) the config: aliases, `Include` files, `ProxyJump`,
`IdentityFile`, custom `Port`, a wildcard `Host *` block, a `Match` block.

## Results

Mark each step **PASS**, **FAIL** or **N/A**, and note anything unexpected. A FAIL needs an
issue or a fix commit reference.

### 1. Host discovery and search

| # | Step | Expected | Result | Notes |
|---|---|---|---|---|
| 1.1 | Start the app | All concrete aliases listed, including those from `Include`d files; `Host *` and patterns not listed | | |
| 1.2 | Type part of an alias; use ↑/↓, Enter | List filters; Enter connects the selection | | |
| 1.3 | Select a ProxyJump / custom-port host | Details line shows the effective `user@host:port` | | |
| 1.4 | Choose a custom config file via the path at the bottom; restart the app | Hosts from that file listed; choice remembered; picking the default reverts | | |
| 1.5 | Open two hosts (A, then B); restart the app | B, then A listed first above a separator line; other hosts in config order | | |

### 2. Embedded terminal and files side by side

| # | Step | Expected | Result | Notes |
|---|---|---|---|---|
| 2.1 | Connect to a host | Terminal opens inside the app; the Files view in the right side bar lists the host; no external console window | | |
| 2.2 | Compare with `ssh <alias>` in a console | Same user, host, prompt, ProxyJump path, key used | | |
| 2.3 | Connect to a new host (unknown host key) | Host-key prompt shown in the terminal; nothing auto-accepted | | |

### 3. Interactive terminal and Byobu

| # | Step | Expected | Result | Notes |
|---|---|---|---|---|
| 3.1 | Run `vim`, `top`, `less`; type UTF-8 text (e.g. `máy chủ`) | Renders correctly; cursor, colours, scrolling OK | | |
| 3.2 | Start `byobu`; press F2, F3, F4, F6, F7, F8, F9 | New window, prev/next window, detach, scrollback, rename, config menu as Byobu defines | | |
| 3.3 | Resize the window and drag the divider; run `stty size` / watch Byobu redraw | Remote size follows the pane | | |
| 3.4 | Copy (Ctrl+Shift+C / selection + Ctrl+C) and paste (Ctrl+V) | Clipboard works; Ctrl+C without selection interrupts | | |
| 3.4b | In `byobu` (tmux backend, mouse on), drag-select text without pressing Ctrl+C; paste into Notepad | The selection is pasted, including Unicode and several lines | | |
| 3.5 | F6 to detach, Reconnect, run `byobu` | Existing session reattached; app does not claim the old process survived | | |

### 4. File browsing and transfers

Prepare files locally and record `certutil -hashfile <file> SHA256`; compare with `sha256sum` on
the server (and the reverse for downloads).

| # | Step | Expected | Result | Notes |
|---|---|---|---|---|
| 4.1 | Browse folders while a command runs in the terminal | Listing works without disturbing the terminal | | |
| 4.2 | Upload a ≥100 MB file | Checksums match | | |
| 4.3 | Upload `máy chủ [1].txt` and a folder `my folder/sub dir/` | Names and checksums match | | |
| 4.4 | Download a file and a folder with spaces/Unicode names | Checksums match; no `.part` left behind | | |
| 4.5 | Cancel a large download; cancel a large upload | Download leaves nothing locally; upload may leave a partial remote file (documented) | | |
| 4.6 | Transfers through the ProxyJump host | Same results as direct | | |
| 4.7 | Password-only host: open the tab; type a wrong password, then the right one | File pane asks, says the wrong one was not accepted, then lists; later listings and transfers do not ask again; closing and reopening the tab asks again | | |
| 4.8 | New host (unknown key) in the file pane: Cancel, then Refresh and Accept | Cancel: host-key error, nothing written to known_hosts; Accept: fingerprint shown, listing works | | |
| 4.9 | Password-only host: tick **Save password**, close and reopen the tab; then change the saved credential to a wrong value in Credential Manager and Refresh | Reopened tab lists without asking; `simter:<alias>:...` appears under Windows Credentials; a wrong saved password is removed and the dialog asks again | | |
| 4.10 | Password-only host: open the tab, enter the password once, then Reconnect the terminal and list files | One dialog for the tab; the terminal logs in and later reconnects and listings do not ask again | | |
| 4.11 | Through the ProxyJump host, open several folders in a row; on the server watch `ps -ef \| grep sftp-server` | Each folder opens without a new connection delay; one `sftp-server` per tab; it ends when the tab closes | | |
| 4.12 | Host whose config sets `RemoteCommand byobu` / `RequestTTY yes` | Files list normally; no Byobu session is started by the file pane | | |
| 4.13 | Browse, then drop the network (or `kill` the tab's `sftp-server`), then Refresh | At most one clear error (a dropped network times out after 30 s); the next Refresh reconnects and lists | | |

### 4b. Right-click menu and delete

Use a disposable folder on the server; check the results there with `ls -la`.

| # | Step | Expected | Result | Notes |
|---|---|---|---|---|
| 4b.1 | Right-click empty space below the list; then right-click a file | Empty space: Upload files…, Upload folder…; item: Download…, Delete…; no upload/download buttons in the pane | | |
| 4b.2 | Select an item with the arrow keys, press the context-menu key or Shift+F10; navigate with Up/Down, Escape | Menu opens beside the selected item; Escape closes it and focus returns to the list; F2–F9 still reach Byobu in the terminal | | |
| 4b.3 | Delete… a file, press Escape (or Cancel) | Warning names the file and path and says it cannot be undone; file still on the server | | |
| 4b.4 | Delete… `my folder` with nested sub-folders and `máy chủ.txt`, confirm | Warning says its contents go too; folder gone; listing refreshes | | |
| 4b.5 | Delete… a symlink to a folder (`ln -s /etc/ssh link`), confirm | Only the link is removed; `/etc/ssh` untouched | | |
| 4b.6 | Delete… a folder containing a root-owned file (no permission) | Error says not deleted and part of the folder may be gone; Refresh shows what is left | | |

### 5. Conflicts

| # | Step | Expected | Result | Notes |
|---|---|---|---|---|
| 5.1 | Upload a file that already exists; choose Skip, then Cancel | Remote file unchanged (checksum) | | |
| 5.2 | Same, choose Overwrite | Remote file replaced | | |
| 5.3 | Download onto an existing local file/folder; Skip / Overwrite | No change on Skip; replaced/merged on Overwrite | | |

### 6. Errors

| # | Step | Expected | Result | Notes |
|---|---|---|---|---|
| 6.1 | Wrong key / no agent key | Terminal shows auth failure; file pane says authentication failed | | |
| 6.2 | Changed host key on the server | Clear host-key error; nothing accepted | | |
| 6.3 | Unreachable host / wrong port | Red tab, exit 255; file pane says unreachable; app stays responsive | | |
| 6.4 | Rename `sftp.exe` away / remove OpenSSH client | Clear "not found" message | | |
| 6.5 | Open a folder without read permission; upload into a read-only folder | Permission-denied message | | |

### 7. Config editing

| # | Step | Expected | Result | Notes |
|---|---|---|---|---|
| 7.1 | Hash the config files; use the app (browse, connect, Open config without saving) | Hashes unchanged | | |
| 7.2 | Open config on a host from an `Include`d file | Editor opens that file at the `Host` line (VS Code) | | |
| 7.3 | Add an alias in the editor, save, press Refresh | New alias appears without restarting | | |

### 8. No secrets persisted

| # | Step | Expected | Result | Notes |
|---|---|---|---|---|
| 8.1 | Inspect `%APPDATA%\dev.simter.desktop\` and `%LOCALAPPDATA%\dev.simter.desktop\` | Only `prefs.json` and WebView2 data; no keys, passwords or host settings (`prefs.json` may list recently opened aliases). Windows Credentials has `simter:` entries only for passwords saved with the checkbox | | |

### 9. Two tabs

| # | Step | Expected | Result | Notes |
|---|---|---|---|---|
| 9.1 | Connect to two hosts; type in each; browse each pane | Independent; switching tabs switches the Files view to that tab's host and folder | | |
| 9.2 | Transfer in one tab while typing in the other | Both keep working | | |
| 9.3 | Close one tab | The other terminal and pane keep working | | |
| 9.4 | Collapse Hosts, then Files; drag the dividers; hide the side bar with the panel button; restart the app | Views collapse/expand, terminal widens and redraws, F2–F9 still reach Byobu; layout is the same after restart | | |

## Deviations from the spec

List anything that does not meet `spec.md`, e.g. transfers show state rather than byte progress.

## Verdict

ACCEPTED / NOT ACCEPTED — with the failing step numbers.
