# Simter (Simple Terminal) — Specification (MVP)

## 1. Goal

Build a lightweight **Windows desktop GUI** that removes two recurring frictions from the existing workflow:

1. **Finding and connecting to SSH hosts:** users should not need to remember host aliases, search manually through SSH config, or open VS Code to locate/edit a host entry.
2. **Transferring files:** users should be able to browse remote files and upload/download between Windows and a selected Linux server without manually constructing `scp`/`sftp` commands.

The existing **OpenSSH → Byobu/tmux on the server** behavior must remain intact. The application is a **unified SSH workspace**: embedded interactive terminal and remote SFTP file browser together in one window, plus SSH host selection. It must NOT launch a separate terminal window for the normal workflow. It is not a new SSH protocol implementation or a replacement for Byobu/tmux.

## 2. Intended environment and assumptions

- Client OS: Windows 10/11.
- Local prerequisite: Windows OpenSSH client. Windows Terminal is not required for the in-app workflow.
- The GUI must include an embedded terminal backed by a real PTY/ConPTY-compatible process (or equivalently correct terminal transport), running the system OpenSSH client. The agent may select a maintained terminal rendering component.
- Remote hosts: SSH-accessible Linux servers, possibly through jump hosts.
- Authentication and trust configuration are managed by OpenSSH (keys, `ssh-agent`, `known_hosts`, password prompts, etc.).
- Default SSH config entry point: `%USERPROFILE%\.ssh\config`.
- Existing Byobu/tmux sessions are managed on the remote server, not by this app.

## 3. MVP user workflows

### W1 — Find and connect

1. Launch the GUI and see saved SSH host aliases from the configured SSH config.
2. Search hosts quickly by alias; optionally show user, hostname, and port when resolvable.
3. Select a host and press Enter or click **Connect**.
4. The selected host opens as a **tab within the same application window**. That tab contains an embedded interactive SSH terminal, connected using the selected **alias** (`ssh my-server`), so OpenSSH applies the existing config. No independent Windows Terminal, PowerShell, or console window appears.
5. A **remote file browser is visible alongside the terminal**, in the same tab, with a resizable splitter. The user can switch focus between panes without disconnecting.
6. Keyboard input and terminal escape sequences must reach the remote shell correctly, including Byobu F2–F9; application shortcuts must not steal these while terminal has focus.
7. Closing/reopening a GUI tab does not claim to preserve a raw SSH process. Persistent remote work remains Byobu/tmux responsibility; reconnect should make it straightforward to attach to an existing Byobu session.

### W2 — Browse and transfer files

1. Connect to a host; its **Files** pane is shown beside the live SSH terminal, not in a separate application window.
2. Browse remote directories, including parent navigation, refresh, and file/folder listing; keep terminal and file panes usable at the same time.
3. Select a Windows file/folder and upload to a chosen remote directory; select a remote file/folder and download to a chosen local directory.
4. See transfer progress or at minimum a clear in-progress state, completion/failure result, and useful error text.
5. Transfers must use SSH-based mechanisms with the user's existing OpenSSH configuration. Do not introduce a separate password/key store.

### W3 — Inspect and edit host definitions

1. From a selected host, offer **Open SSH config** at the corresponding config file, ideally near the relevant `Host` entry.
2. Allow users to add or edit aliases and connection directives using an editor or a basic UI, while preserving other config content and comments.
3. Refresh the host list after external edits without requiring an application restart.

**MVP priority:** W1 and W2 must work end-to-end. W3 may initially use the user's configured external editor rather than implementing a full GUI configuration editor.

## 4. Functional requirements

### FR-1: SSH host discovery

- Use the existing OpenSSH config as the **source of truth**; no duplicate database of SSH connection settings.
- Support the default config location and a user-selectable config path.
- Respect `Include` directives and nested file locations when discovering selectable host aliases.
- Present concrete aliases declared in `Host` patterns; do **not** present wildcard-only patterns (`*`, `*.example.com`, negated patterns) as directly connectable hosts.
- Use `ssh -G <alias>` (or an equivalently correct OpenSSH resolution mechanism) to inspect effective connection options where necessary; do not treat a naive parser as the final interpretation of SSH config.
- Display parsing/discovery errors without silently dropping all working hosts.
- For host entries that cannot be mapped unambiguously to one source location, **Open config** can open the main config file rather than pretending an exact mapping exists.

### FR-2: Host search and launch

- Support responsive case-insensitive filtering of host aliases; basic fuzzy matching is desirable but not required for MVP.
- Provide keyboard-first selection and Enter-to-connect in the app, creating/focusing an in-app workspace tab.
- Pass the original alias to `ssh`, rather than assembling `user@hostname -p ...` and losing `ProxyJump`, `IdentityFile`, `Match`, and other config semantics.
- Correctly handle Windows paths, spaces, quoting, and special characters when starting the system OpenSSH process inside a PTY. Avoid shell-string interpolation of user-controlled host names.
- Report clearly if `ssh.exe` or the required terminal runtime is unavailable.

### FR-3: Embedded terminal and workspace layout

- The primary screen is a single application window with a searchable host sidebar/list and per-host tabs.
- Each connected tab displays an **embedded SSH terminal beside a remote file pane**; the divider is resizable, and the layout does not open a secondary SSH terminal window.
- Support interactive TTY applications (minimum: bash, vim, top, Byobu/tmux) with correct terminal resize behavior, ANSI/VT escape sequences, UTF-8, scrolling, copy/paste, cursor handling, and prompt input.
- Use Windows ConPTY (or an equivalent tested PTY mechanism) to host the installed OpenSSH executable, and a maintained terminal renderer. Do not implement a terminal escape-sequence parser from scratch.
- Forward Byobu F2–F9 and standard modifier combinations reliably when terminal has focus. If app shortcuts conflict, remap/disable them; do not silently intercept function keys.
- SSH terminal and SFTP pane are **associated with the same selected SSH host alias**. They may use separate underlying SSH connections as required by SFTP, but neither pane may launch a separate visible terminal window.
- Show connection state clearly; on reconnect, allow the user to resume their existing Byobu/tmux session through the normal remote mechanism. Do not misrepresent local tab restoration as remote process persistence.
- Support at least two connected host tabs, independently switchable without dropping the others.

### FR-4: File management and transfer

- Browse/list remote files over an SSH-based file-transfer mechanism (prefer SFTP where available).
- Support upload, download, refresh, navigation, and clear error handling.
- Preserve file contents and handle common filenames including spaces and Unicode.
- Do not silently overwrite an existing destination file; require explicit confirmation or a clearly defined conflict policy.
- Support key authentication and SSH config options (notably `ProxyJump`, custom ports, and identity files) through the underlying SSH toolchain.
- If an OpenSSH config feature cannot be honored by the chosen transfer implementation, report it explicitly rather than silently using an incorrect connection.
- Delete (a folder recursively) is the only destructive remote operation. It must ask for explicit confirmation that names the item, says that a folder's contents go too, and warns that it cannot be undone; links are deleted, never followed. Do not implement chmod or rename.

### FR-5: Configuration and local state

- Store only app-specific preferences (e.g., window layout, optional display groups, recent hosts, external editor choice) separately from the SSH config.
- Never store plaintext passwords or copy private keys into application state.
- Do not rewrite, normalize, or reorder SSH config merely to read it.
- When editing config, preserve existing formatting/comments and avoid corrupting included files; validate and report errors before replacing user-authored content.

## 5. Non-goals

- Implementing a terminal emulator/parser from scratch. **Embedding an existing terminal rendering component is required**, not a non-goal.
- Reimplementing SSH, SFTP cryptography, SSH agent, or credential storage.
- Replacing Byobu/tmux or managing persistent server processes.
- Synchronizing SSH credentials/config between devices.
- Full-featured remote IDE, shell automation framework, or multi-host command broadcaster.
- Remote file editing, drag-and-drop, directory synchronization, and transfers with pause/resume guarantees are optional future enhancements, not MVP blockers.

## 6. Quality and safety constraints

- **No terminal keybinding conflicts:** Byobu receives F2–F9 via the embedded terminal; no application-global hooks may steal these keys while terminal is focused.
- **No surprises in config:** existing OpenSSH CLI commands continue to work unchanged before and after using the app.
- **Security:** honor SSH host-key verification; never auto-accept unknown/changed host keys; never log private keys, passwords, or tokens.
- **Failure isolation:** an unavailable remote server or failed transfer must not crash the GUI or block browsing other hosts.
- **Responsiveness:** SSH discovery, connection checks, and file operations must not freeze the main UI thread.
- **Transparency:** report the operation attempted and meaningful failure diagnostics, without exposing secrets.

## 7. Acceptance criteria

A build is acceptable only when all of the following are demonstrated:

1. With a representative existing SSH config (aliases, `Include`, `ProxyJump`, `IdentityFile`, custom port), the GUI discovers intended aliases and provides working search.
2. Selecting a host opens an **embedded terminal in the same application** and displays its remote files **alongside it**. No additional SSH terminal window opens. The resolved SSH connection behaves like manually running `ssh <alias>`.
3. In the embedded SSH terminal, interactive bash/vim/top and Byobu's F2/F3/F4/F6/F7/F8/F9 work as expected; resizing the pane updates the remote terminal size.
4. File browsing works **simultaneously with the connected terminal** on a reachable Linux test host; upload and download pass a byte-for-byte checksum comparison, including a filename containing spaces or Unicode.
5. Existing destination-file conflicts never cause silent overwrites.
6. Failed authentication, host-key errors, unreachable hosts, missing local prerequisites, and denied remote file permissions show intelligible errors, without freezing/crashing the app.
7. Opening/editing SSH config does not modify unrelated directives/comments; external changes can be refreshed.
8. No private keys, credentials, or plaintext passwords are persisted by the app.
9. Two simultaneous SSH host tabs remain independently interactive, with each file pane targeting the correct host; closing one tab does not disrupt the other.

## 8. Required validation approach

- Automated tests for host discovery, selection/filtering, command argument construction, path handling, and error cases.
- Integration tests against a controlled SSH test server (including SFTP operations and a jump-host scenario where feasible).
- **Mandatory manual validation on a real Windows + Linux testbed**, using the **embedded** terminal and a running Byobu session; mocks alone are insufficient for acceptance. Test F2–F9, TUI rendering, pane resize, clipboard, and concurrent SFTP browsing/transfers.
- Record a short test report listing environment, steps, pass/fail results, and any deviations from the spec.

## 9. Implementation discretion and deliverables

The coding agent may choose the GUI toolkit, maintained embedded terminal component, PTY integration, internal modules, and transfer implementation. Prefer existing, maintained libraries and the installed OpenSSH tools over writing protocol handling from scratch. **A host launcher that opens Windows Terminal externally does not satisfy this spec.** Do not add extra product features unless needed to satisfy the acceptance criteria.

Deliver:
- Source code and repeatable Windows setup/build instructions.
- Runnable Windows app (or reproducible packaging procedure).
- Tests and real-testbed validation instructions/report template.
- `README.md` documenting prerequisites, basic operations, supported SSH config behavior, and known limitations.

When a requirement cannot be met exactly, flag the discrepancy explicitly; do not silently implement an approximation.
