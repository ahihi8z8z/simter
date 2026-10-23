import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Details } from "./hostList";

// Mirrors the serialized types in src-tauri/src/hosts and commands.rs.

export interface Location {
  file: string;
  line: number;
}

export interface Host {
  alias: string;
  locations: Location[];
  conditional: boolean;
}

export interface Issue {
  severity: "error" | "warning";
  file: string;
  line: number | null;
  message: string;
}

export interface HostList {
  configPath: string;
  defaultConfigPath: string;
  hosts: Host[];
  issues: Issue[];
}

export function listHosts(configPath?: string): Promise<HostList> {
  return invoke<HostList>("list_hosts", { configPath: configPath ?? null });
}

/** Mirrors prefs.rs. `editor` is only set by hand in prefs.json. */
export interface Prefs {
  configPath?: string;
  layout?: Layout;
  editor?: string;
  /** Host aliases opened in the app, most recent first. */
  recentHosts?: string[];
}

export function loadPrefs(): Promise<Prefs> {
  return invoke<Prefs>("prefs_load");
}

export function saveConfigPathPref(configPath: string | undefined): Promise<void> {
  return invoke("prefs_set_config_path", { configPath: configPath ?? null });
}

/** Mirrors prefs.rs `Layout`: the side bar with hosts above files. */
export interface Layout {
  sidebarWidth?: number;
  filesHeight?: number;
  sidebarHidden?: boolean;
  hostsCollapsed?: boolean;
  filesCollapsed?: boolean;
}

export function saveLayoutPref(layout: Layout): Promise<void> {
  return invoke("prefs_set_layout", { layout });
}

/** Records that `alias` was opened; resolves to the recent hosts, most recent first. */
export function recordRecentHost(alias: string): Promise<string[]> {
  return invoke<string[]>("prefs_record_host", { alias });
}

/** Opens a config file in an external editor (VS Code at the line, else Notepad). */
export function openConfig(file: string, line: number | null): Promise<void> {
  return invoke("open_config", { file, line });
}

/** Effective connection details from `ssh -G <alias>`. */
export function hostDetails(alias: string, configPath?: string): Promise<Details> {
  return invoke<Details>("host_details", { alias, configPath: configPath ?? null });
}

export interface Exit {
  code: number | null;
  error: string | null;
}

export interface TerminalHandlers {
  output(data: Uint8Array): void;
  exit(exit: Exit): void;
}

/** Starts `ssh <alias>` in a pseudoconsole and returns its terminal id. */
export function openTerminal(
  session: string,
  alias: string,
  cols: number,
  rows: number,
  handlers: TerminalHandlers,
  configPath?: string,
): Promise<number> {
  // Output arrives as raw bytes, then the exit status as JSON, in order.
  const events = new Channel<ArrayBuffer | Exit>((event) => {
    if (event instanceof ArrayBuffer) handlers.output(new Uint8Array(event));
    else handlers.exit(event);
  });
  return invoke<number>("terminal_open", { session, alias, configPath: configPath ?? null, cols, rows, events });
}

export function writeTerminal(id: number, data: string): Promise<void> {
  return invoke("terminal_write", { id, data });
}

export function resizeTerminal(id: number, cols: number, rows: number): Promise<void> {
  return invoke("terminal_resize", { id, cols, rows });
}

export function closeTerminal(id: number): Promise<void> {
  return invoke("terminal_close", { id });
}

// Mirrors src-tauri/src/sftp.rs.

export interface Entry {
  name: string;
  kind: "dir" | "file" | "link" | "other";
  size: number;
  modified: string;
}

export interface Listing {
  path: string;
  entries: Entry[];
  unparsed: string[];
}

export interface SftpError {
  kind:
    | "unavailable"
    | "config"
    | "unreachable"
    | "hostKey"
    | "auth"
    | "noSftp"
    | "remote"
    | "unsupportedPath"
    | "timeout"
    | "exists"
    | "cancelled"
    | "local"
    | "other";
  message: string;
}

/**
 * Lists `path` on `alias` over SFTP (the login directory when omitted). Rejects with an SftpError.
 * `session` is the tab whose remembered passwords answer ssh's prompts; the tab keeps one SFTP
 * connection open for browsing until `closeRemote`.
 */
export function listRemote(session: string, alias: string, path?: string, configPath?: string): Promise<Listing> {
  return invoke<Listing>("sftp_list", { session, alias, path: path ?? null, configPath: configPath ?? null });
}

/**
 * Permanently deletes `name` in `remoteDir`; a folder goes with everything in it, and a link is
 * deleted without following it. Rejects with an SftpError.
 */
export function deleteRemote(session: string, alias: string, remoteDir: string, name: string, configPath?: string): Promise<void> {
  return invoke("sftp_delete", { session, alias, remoteDir, name, configPath: configPath ?? null });
}

/** Closes the tab's SFTP browsing connection. */
export function closeRemote(session: string): Promise<void> {
  return invoke("sftp_close", { session });
}

/** Mirrors sftp.rs `Size`: bytes, and for a folder the regular files in it (null for a file). */
export interface Size {
  bytes: number;
  files: number | null;
}

/**
 * Mirrors commands.rs `Progress`: the size of the copy so far, and the total once measured (null
 * until then, or if it cannot be).
 */
export interface Progress {
  done: Size;
  total: Size | null;
}

/**
 * Uploads the local file or folder `local` into `remoteDir`. Rejects with an
 * SftpError of kind "exists" if the target exists and `overwrite` is false.
 */
export function uploadPath(
  id: number,
  session: string,
  alias: string,
  local: string,
  remoteDir: string,
  overwrite: boolean,
  onProgress: (progress: Progress) => void,
  configPath?: string,
): Promise<void> {
  const progress = new Channel<Progress>(onProgress);
  return invoke("sftp_upload", { id, session, alias, local, remoteDir, overwrite, configPath: configPath ?? null, progress });
}

/** Downloads `name` from `remoteDir` into `localDir`; conflicts as for uploadPath. */
export function downloadEntry(
  id: number,
  session: string,
  alias: string,
  remoteDir: string,
  name: string,
  localDir: string,
  overwrite: boolean,
  onProgress: (progress: Progress) => void,
  configPath?: string,
): Promise<void> {
  const progress = new Channel<Progress>(onProgress);
  return invoke("sftp_download", {
    id,
    session,
    alias,
    remoteDir,
    name,
    localDir,
    overwrite,
    configPath: configPath ?? null,
    progress,
  });
}

/** Stops a running transfer; it then rejects with kind "cancelled". */
export function cancelTransfer(id: number): Promise<void> {
  return invoke("transfer_cancel", { id });
}

/** Mirrors askpass.rs `Question`: a prompt from ssh for the user. */
export interface Question {
  id: number;
  alias: string;
  prompt: string;
  /** A password or passphrase, rather than a yes/no question. */
  secret: boolean;
  /** ssh rejected the previous answer to this prompt. */
  retry: boolean;
  /** A problem reading or removing a saved password. */
  error: string | null;
}

export function onAskpass(handler: (question: Question) => void): Promise<UnlistenFn> {
  return listen<Question>("askpass", (event) => handler(event.payload));
}

/**
 * Answers question `id`; null cancels it. With `save`, the password is also saved in
 * Windows Credential Manager; rejects with the error if that fails (ssh still gets the answer).
 */
export function askpassReply(id: number, answer: string | null, save: boolean): Promise<void> {
  return invoke("askpass_reply", { id, answer, save });
}

/** Forgets the passwords remembered for a closed tab. */
export function endAskpassSession(session: string): Promise<void> {
  return invoke("askpass_end", { session });
}
