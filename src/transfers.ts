// Transfer bookkeeping for the file pane, kept free of DOM and Tauri for unit tests.

import type { Progress, SftpError } from "./api";
import { formatSize } from "./remotePath";

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

/**
 * What a transfer row shows: the bar's fraction (null for an unknown total) and its text, with
 * the current `speed` in bytes per second when known. Counts are capped, since merging into an
 * existing folder counts files already there.
 */
export function describeProgress(
  { done, total }: Progress,
  speed: number | null = null,
): { fraction: number | null; text: string } {
  const rate = speed === null ? [] : [`${formatSize(Math.round(speed))}/s`];
  if (total === null || total.bytes === 0) {
    if (done.bytes === 0 && rate.length === 0) return { fraction: null, text: "Running…" };
    return { fraction: null, text: [formatSize(done.bytes), ...rate].join(" · ") };
  }
  const shown = Math.min(done.bytes, total.bytes);
  const fraction = shown / total.bytes;
  const parts = [`${Math.floor(fraction * 100)}%`, `${formatSize(shown)} of ${formatSize(total.bytes)}`];
  if (total.files !== null) {
    // Files are copied one at a time, so the last one present is still being written.
    const present = done.files ?? 0;
    const finished = shown === total.bytes ? present : Math.max(present - 1, 0);
    parts.push(`${Math.min(finished, total.files)}/${total.files} files`);
  }
  return { fraction, text: [...parts, ...rate].join(" · ") };
}

/** Transfer rate in bytes per second, averaged over the last few seconds of measurements. */
export class SpeedMeter {
  private samples: { at: number; bytes: number }[] = [];

  constructor(private readonly windowMs = 3000) {}

  /** Adds a measurement of `bytes` done at `at` ms; null until two measurements are apart. */
  sample(bytes: number, at: number): number | null {
    const last = this.samples.at(-1);
    // Less than before: the copy restarted, e.g. after choosing Overwrite.
    if (last !== undefined && bytes < last.bytes) this.samples = [];
    this.samples.push({ at, bytes });
    while (this.samples.length > 2 && at - this.samples[1].at >= this.windowMs) this.samples.shift();
    const first = this.samples[0];
    return at > first.at ? ((bytes - first.bytes) * 1000) / (at - first.at) : null;
  }
}

/** The last component of a Windows or remote path. */
export function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  return trimmed.slice(Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\")) + 1) || trimmed;
}
