// Transfer bookkeeping for the file pane, kept free of DOM and Tauri for unit tests.

import type { SftpError } from "./api";

export type Choice = "overwrite" | "skip" | "cancel";
export type Outcome = "done" | "skipped" | "cancelled";

function isExists(e: unknown): e is SftpError {
  return typeof e === "object" && e !== null && (e as SftpError).kind === "exists";
}

/**
 * Runs `attempt` without overwriting. If the destination exists, asks the
 * user and retries with overwrite only when they choose to. Other errors
 * propagate.
 */
export async function withConflictCheck(
  attempt: (overwrite: boolean) => Promise<void>,
  ask: (message: string) => Promise<Choice>,
): Promise<Outcome> {
  try {
    await attempt(false);
    return "done";
  } catch (e) {
    if (!isExists(e)) throw e;
    const choice = await ask(e.message);
    if (choice !== "overwrite") return choice === "skip" ? "skipped" : "cancelled";
    await attempt(true);
    return "done";
  }
}

/** The last component of a Windows or remote path. */
export function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  return trimmed.slice(Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\")) + 1) || trimmed;
}
