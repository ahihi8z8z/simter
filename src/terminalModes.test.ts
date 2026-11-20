import { describe, expect, it } from "vitest";
import { Terminal } from "@xterm/xterm";
import { RESET_REMOTE_MODES } from "./terminalModes";

function write(term: Terminal, data: string): Promise<void> {
  return new Promise((resolve) => term.write(data, resolve));
}

describe("RESET_REMOTE_MODES", () => {
  it("undoes the modes tmux with mouse on leaves behind", async () => {
    const term = new Terminal({ allowProposedApi: true });
    // What tmux enables while attached: alternate screen, SGR any-event mouse tracking,
    // bracketed paste, focus events, application cursor keys, hidden cursor.
    await write(term, "\x1b[?1049h\x1b[?1003h\x1b[?1006h\x1b[?2004h\x1b[?1004h\x1b[?1h\x1b[?25l");
    expect(term.modes.mouseTrackingMode).toBe("any");
    expect(term.buffer.active.type).toBe("alternate");

    await write(term, RESET_REMOTE_MODES);

    expect(term.modes.mouseTrackingMode).toBe("none");
    expect(term.modes.bracketedPasteMode).toBe(false);
    expect(term.modes.sendFocusMode).toBe(false);
    expect(term.modes.applicationCursorKeysMode).toBe(false);
    expect(term.buffer.active.type).toBe("normal");
    term.dispose();
  });

  it("keeps the scrollback", async () => {
    const term = new Terminal({ rows: 5, scrollback: 100 });
    await write(term, Array.from({ length: 20 }, (_, i) => `line ${i}`).join("\r\n"));
    await write(term, RESET_REMOTE_MODES);
    expect(term.buffer.active.getLine(0)?.translateToString(true)).toBe("line 0");
    term.dispose();
  });
});
