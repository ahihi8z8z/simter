// Pure helpers for the file pane, kept free of DOM and Tauri for unit tests.

import type { Entry, SftpError } from "./api";

/** Joins a remote directory and an entry name with `/`. */
export function joinPath(dir: string, name: string): string {
  return dir.endsWith("/") ? dir + name : `${dir}/${name}`;
}

/** The parent of an absolute remote path; `/` is its own parent. */
export function parentPath(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const cut = trimmed.lastIndexOf("/");
  return cut <= 0 ? "/" : trimmed.slice(0, cut);
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let value = bytes;
  let unit = -1;
  do {
    value /= 1024;
    unit++;
  } while (value >= 1024 && unit < units.length - 1);
  return `${value.toFixed(1)} ${units[unit]}`;
}

/** Folders first, then by name (natural, case-insensitive). */
export function sortEntries(entries: Entry[]): Entry[] {
  const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
  return [...entries].sort(
    (a, b) => Number(b.kind === "dir") - Number(a.kind === "dir") || collator.compare(a.name, b.name),
  );
}

const hints: Partial<Record<SftpError["kind"], string>> = {
  unavailable: "Install the Windows OpenSSH Client feature.",
  config: "Fix the SSH config, then Refresh.",
  unreachable: "Check the network, HostName, Port and ProxyJump settings.",
  hostKey: "The host key was not accepted, or it has changed. Review it in the terminal, then Refresh.",
  auth: "Check the password, passphrase or key, then Refresh to try again.",
  noSftp: "The server does not offer the SFTP subsystem.",
  timeout: "The server did not answer in time. Refresh to retry.",
};

/** The error text plus what the user can do about it, if anything. */
export function describeSftpError(error: SftpError): { message: string; hint: string | null } {
  return { message: error.message, hint: hints[error.kind] ?? null };
}
