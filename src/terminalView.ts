import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { closeTerminal, openTerminal, resizeTerminal, writeTerminal, type Exit } from "./api";
import { describeExit, type State } from "./connection";
import { clipboardAction } from "./keys";

/**
 * One xterm.js terminal for a host. Each `connect()` runs a fresh
 * `ssh <alias>` in it; the scrollback survives reconnects, the remote shell
 * does not (that is Byobu/tmux's job).
 */
export class TerminalView {
  readonly element: HTMLDivElement;
  private readonly term: Terminal;
  private readonly fit = new FitAddon();
  private readonly resizeObserver: ResizeObserver;
  private opened = false;
  private disposed = false;
  // Bumped per connection, so events from an earlier process are ignored.
  private generation = 0;
  private id: number | null = null;
  private state: State = "disconnected";

  constructor(
    readonly alias: string,
    /** Tab session shared with the file pane, for password prompts. */
    private readonly session: string,
    private readonly onState: (state: State, detail: string) => void,
    private readonly configPath?: string,
  ) {
    this.element = document.createElement("div");
    this.element.className = "terminal";
    this.term = new Terminal({
      fontFamily: '"Cascadia Mono", Consolas, monospace',
      fontSize: 14,
      cursorBlink: true,
      scrollback: 5000,
      rightClickSelectsWord: true,
    });
    this.term.loadAddon(this.fit);
    this.term.attachCustomKeyEventHandler((e) => this.handleKey(e));
    this.term.onData((data) => this.send(data));
    this.term.onResize(({ cols, rows }) => {
      // Fails only after the process has exited, which is reported separately.
      if (this.id !== null) resizeTerminal(this.id, cols, rows).catch(() => {});
    });
    this.resizeObserver = new ResizeObserver(() => this.fitToElement());
  }

  get connected(): boolean {
    return this.state === "connecting" || this.state === "connected";
  }

  /** Runs `ssh <alias>`. The element must be in the document and visible. */
  async connect(): Promise<void> {
    if (this.disposed || this.connected) return;
    if (!this.opened) {
      this.term.open(this.element);
      this.resizeObserver.observe(this.element);
      this.opened = true;
    } else {
      this.term.write("\r\n\x1b[2m[Reconnecting]\x1b[0m\r\n");
    }
    this.fitToElement();
    this.term.focus();
    const generation = ++this.generation;
    this.setState("connecting", `Running ssh ${this.alias}`);
    try {
      const id = await openTerminal(
        this.session,
        this.alias,
        this.term.cols,
        this.term.rows,
        {
          output: (data) => {
            if (generation === this.generation) this.term.write(data);
          },
          exit: (exit) => {
            if (generation === this.generation) this.exited(exit);
          },
        },
        this.configPath,
      );
      if (this.disposed) {
        void closeTerminal(id);
      } else if (generation === this.generation && this.state === "connecting") {
        // Not already exited: the exit event can arrive before the id does.
        this.id = id;
        this.setState("connected", `ssh ${this.alias}`);
      }
    } catch (e) {
      if (generation !== this.generation) return;
      this.setState("failed", String(e));
      this.term.write(`\r\n\x1b[31m${String(e)}\x1b[0m\r\n`);
    }
  }

  /** Re-fits and focuses after the tab becomes visible. */
  show(): void {
    this.fitToElement();
    this.term.focus();
  }

  /** Ends the ssh process (if any) and removes the terminal. */
  dispose(): void {
    this.disposed = true;
    this.generation++;
    this.resizeObserver.disconnect();
    if (this.id !== null) void closeTerminal(this.id);
    this.id = null;
    this.term.dispose();
    this.element.remove();
  }

  private setState(state: State, detail: string): void {
    this.state = state;
    this.onState(state, detail);
  }

  private send(data: string): void {
    if (this.id === null) return;
    // Fails only after the process has exited, which is reported separately.
    writeTerminal(this.id, data).catch(() => {});
  }

  private exited(exit: Exit): void {
    this.id = null;
    const outcome = describeExit(exit);
    this.setState(outcome.state, outcome.message);
    this.term.write(`\r\n\x1b[2m[${outcome.message}]\x1b[0m\r\n`);
  }

  private fitToElement(): void {
    if (this.element.offsetWidth > 0 && this.element.offsetHeight > 0) this.fit.fit();
  }

  private handleKey(e: KeyboardEvent): boolean {
    switch (clipboardAction(e, this.term.hasSelection())) {
      case "copy":
        e.preventDefault();
        navigator.clipboard.writeText(this.term.getSelection()).catch((err) => console.error("copy failed", err));
        this.term.clearSelection();
        return false;
      case "paste":
        // Let the browser fire its native paste event, which xterm.js turns
        // into input (with bracketed-paste markers when the remote enables them).
        return false;
      default:
        return true;
    }
  }
}
