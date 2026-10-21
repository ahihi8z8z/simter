import type { Exit } from "./api";

export type State = "connecting" | "connected" | "disconnected" | "failed";

export interface Outcome {
  state: "disconnected" | "failed";
  message: string;
}

/**
 * Explains why `ssh` ended. OpenSSH exits with 255 for its own errors
 * (unreachable host, authentication, host key, config); any other code is
 * the remote command's status passed through.
 */
export function describeExit(exit: Exit): Outcome {
  if (exit.error !== null) return { state: "failed", message: exit.error };
  switch (exit.code) {
    case 0:
      return { state: "disconnected", message: "Session ended" };
    case 255:
      return {
        state: "failed",
        message: "ssh could not connect or authenticate (exit 255); see the terminal for details",
      };
    default:
      return { state: "disconnected", message: `Remote shell exited with code ${exit.code}` };
  }
}
