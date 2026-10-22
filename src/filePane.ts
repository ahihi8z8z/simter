import { open } from "@tauri-apps/plugin-dialog";
import {
  cancelTransfer,
  deleteRemote,
  downloadEntry,
  listRemote,
  uploadPath,
  type Entry,
  type Progress,
  type SftpError,
} from "./api";
import { isMenuOpen, showMenu, type MenuItem } from "./contextMenu";
import { moveSelection } from "./hostList";
import { deleteWarning, describeSftpError, formatSize, joinPath, parentPath, sortEntries } from "./remotePath";
import { baseName, describeProgress, withConflictCheck, type Choice } from "./transfers";

const icons: Record<Entry["kind"], string> = { dir: "📁", file: "📄", link: "🔗", other: "⚙" };

function button(text: string, title: string): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = text;
  b.title = title;
  return b;
}

// Shared by all tabs: ids for transfer_cancel, and the last download folder.
let nextTransferId = 1;
let lastDownloadDir: string | undefined;

/** Asks whether to replace an existing destination. Escape means Cancel. */
function askConflict(message: string): Promise<Choice> {
  const dialog = document.createElement("dialog");
  dialog.className = "conflict";
  const form = document.createElement("form");
  form.method = "dialog";
  const text = document.createElement("p");
  text.textContent = `${message}. Replace it?`;
  const actions = document.createElement("div");
  actions.className = "conflict-actions";
  const choices: [Choice, string, string][] = [
    ["overwrite", "Overwrite", "Replace the existing file or merge into the existing folder"],
    ["skip", "Skip", "Leave the existing one and continue with the next"],
    ["cancel", "Cancel", "Stop this and the remaining transfers in the batch"],
  ];
  for (const [value, label, title] of choices) {
    const b = button(label, title);
    b.type = "submit";
    b.value = value;
    b.autofocus = value === "skip";
    actions.append(b);
  }
  form.append(text, actions);
  dialog.append(form);
  document.body.append(dialog);
  dialog.showModal();
  return new Promise((resolve) => {
    dialog.addEventListener(
      "close",
      () => {
        resolve((dialog.returnValue || "cancel") as Choice);
        dialog.remove();
      },
      { once: true },
    );
  });
}

/** Asks before a permanent delete. Escape and the default button mean Cancel. */
function confirmDelete(message: string): Promise<boolean> {
  const dialog = document.createElement("dialog");
  dialog.className = "conflict";
  const form = document.createElement("form");
  form.method = "dialog";
  const text = document.createElement("p");
  text.className = "delete-warning";
  text.textContent = message;
  const actions = document.createElement("div");
  actions.className = "conflict-actions";
  const remove = button("Delete", "Delete permanently");
  remove.type = "submit";
  remove.value = "delete";
  remove.className = "danger";
  const cancel = button("Cancel", "Keep it");
  cancel.type = "submit";
  cancel.value = "cancel";
  cancel.autofocus = true;
  actions.append(remove, cancel);
  form.append(text, actions);
  dialog.append(form);
  document.body.append(dialog);
  dialog.showModal();
  return new Promise((resolve) => {
    dialog.addEventListener(
      "close",
      () => {
        resolve(dialog.returnValue === "delete");
        dialog.remove();
      },
      { once: true },
    );
  });
}

interface Job {
  arrow: "↑" | "↓";
  name: string;
  title: string;
  run(id: number, overwrite: boolean, onProgress: (progress: Progress) => void): Promise<void>;
}

/** One line in the transfer list. */
class TransferRow {
  readonly element = document.createElement("li");
  private readonly status = document.createElement("span");
  private readonly bar = document.createElement("progress");
  private readonly action = document.createElement("button");

  constructor(job: Job) {
    this.element.className = "transfer";
    this.element.title = job.title;
    const arrow = document.createElement("span");
    arrow.className = "transfer-arrow";
    arrow.textContent = job.arrow;
    const name = document.createElement("span");
    name.className = "transfer-name";
    name.textContent = job.name;
    this.status.className = "transfer-status";
    this.bar.className = "transfer-bar";
    this.bar.hidden = true;
    this.action.type = "button";
    this.action.className = "transfer-action";
    this.action.hidden = true;
    this.element.append(arrow, name, this.bar, this.status, this.action);
    this.set("waiting", "Waiting");
  }

  start(cancel: () => void): void {
    this.set("running", "Running…");
    // Without a value the bar is indeterminate until the total is known.
    this.bar.removeAttribute("value");
    this.bar.hidden = false;
    this.action.hidden = false;
    this.action.textContent = "Cancel";
    this.action.title = "Stop this transfer";
    this.action.onclick = () => {
      this.action.disabled = true;
      cancel();
    };
  }

  /** Shows the latest measurement. */
  progress(progress: Progress): void {
    const { fraction, text } = describeProgress(progress);
    if (fraction === null) this.bar.removeAttribute("value");
    else this.bar.value = fraction;
    this.status.textContent = text;
    this.status.title = text;
  }

  finish(state: "done" | "skipped" | "cancelled" | "failed", text: string, detail = text): void {
    this.set(state, text);
    this.bar.hidden = true;
    this.status.title = detail;
    this.action.hidden = false;
    this.action.disabled = false;
    this.action.textContent = "×";
    this.action.title = "Remove from the list";
    this.action.onclick = () => this.element.remove();
  }

  private set(state: string, text: string): void {
    this.element.dataset.state = state;
    this.status.textContent = text;
  }
}

/**
 * Remote file browser for one host, over the tab's SFTP connection. Right-click (or the
 * context-menu key) offers upload on empty space and download/delete on an item; keys here act
 * only while the pane has focus.
 */
export class FilePane {
  readonly element: HTMLElement;
  private readonly pathInput: HTMLInputElement;
  private readonly upButton: HTMLButtonElement;
  private readonly refreshButton: HTMLButtonElement;
  private readonly status: HTMLDivElement;
  private readonly list: HTMLUListElement;
  private readonly transfers: HTMLUListElement;
  private path: string | null = null;
  private entries: Entry[] = [];
  private selected = -1;
  // Bumped per request, so a slow earlier listing cannot overwrite a newer one.
  private generation = 0;

  constructor(
    private readonly alias: string,
    /** The tab's askpass session, which remembers passwords typed for it. */
    private readonly session: string,
    private readonly configPath?: string,
  ) {
    this.element = document.createElement("section");
    this.element.className = "file-pane";
    this.element.ariaLabel = `Files on ${alias}`;

    const toolbar = document.createElement("div");
    toolbar.className = "file-toolbar";
    this.upButton = button("↑", "Parent folder (Backspace)");
    this.pathInput = document.createElement("input");
    this.pathInput.className = "file-path";
    this.pathInput.spellcheck = false;
    this.pathInput.autocomplete = "off";
    this.pathInput.title = "Remote folder; type a path and press Enter";
    this.refreshButton = button("⟳", "Refresh");
    toolbar.append(this.upButton, this.pathInput, this.refreshButton);

    this.status = document.createElement("div");
    this.status.className = "file-status";
    this.list = document.createElement("ul");
    this.list.className = "file-list";
    this.list.role = "listbox";
    this.list.tabIndex = 0;
    this.list.title = "Right-click for upload, download and delete";

    this.transfers = document.createElement("ul");
    this.transfers.className = "transfers";
    this.transfers.ariaLabel = "Transfers";
    this.element.append(toolbar, this.status, this.list, this.transfers);

    // A right-click starts with a pointerdown; the context-menu key and Shift+F10 do not.
    let fromPointer = false;
    this.list.addEventListener("pointerdown", (e) => (fromPointer = e.button === 2));
    this.list.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      const pointer = fromPointer;
      fromPointer = false;
      if (!pointer) {
        // Keyboard: handled on keydown, unless the WebView did not deliver that key.
        if (!isMenuOpen()) this.openMenuForKeyboard();
        return;
      }
      const item = (e.target as Element).closest<HTMLElement>(".file");
      const index = item ? Number(item.dataset.index) : -1;
      if (index >= 0) this.select(index);
      this.openMenu(index, e.clientX, e.clientY);
    });

    this.upButton.addEventListener("click", () => this.up());
    this.refreshButton.addEventListener("click", () => void this.load(this.path ?? undefined));
    this.pathInput.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        void this.load(this.pathInput.value.trim() || undefined);
      } else if (e.key === "Escape") {
        this.pathInput.value = this.path ?? "";
      }
    });
    this.list.addEventListener("keydown", (e) => this.handleListKey(e));
  }

  /** Lists `path`, or the login directory when omitted. */
  async load(path?: string): Promise<void> {
    const generation = ++this.generation;
    this.setBusy(true);
    this.showStatus(path ? `Listing ${path}…` : "Listing home folder…");
    try {
      const listing = await listRemote(this.session, this.alias, path, this.configPath);
      if (generation !== this.generation) return;
      this.path = listing.path;
      this.pathInput.value = listing.path;
      this.entries = sortEntries(listing.entries);
      this.selected = this.entries.length ? 0 : -1;
      this.render();
      const count = `${this.entries.length} ${this.entries.length === 1 ? "item" : "items"}`;
      if (listing.unparsed.length) {
        this.showStatus(`${count}; ${listing.unparsed.length} line(s) could not be read`, "warning");
        this.status.title = listing.unparsed.join("\n");
      } else {
        this.showStatus(count);
      }
    } catch (e) {
      if (generation !== this.generation) return;
      const { message, hint } = describeSftpError(e as SftpError);
      // Keep the previous listing visible; the path box returns to it.
      this.pathInput.value = this.path ?? "";
      this.showStatus(hint ? `${message}\n${hint}` : message, "error");
    } finally {
      if (generation === this.generation) this.setBusy(false);
    }
  }

  private up(): void {
    if (this.path !== null && this.path !== "/") void this.load(parentPath(this.path));
  }

  private open(index: number): void {
    const entry = this.entries[index];
    if (!entry || this.path === null) return;
    // A link may point to a folder; if not, sftp reports it.
    if (entry.kind === "dir" || entry.kind === "link") void this.load(joinPath(this.path, entry.name));
  }

  private async upload(folder: boolean): Promise<void> {
    const remoteDir = this.path;
    if (remoteDir === null) return;
    const picked = await open({
      title: folder ? `Upload a folder to ${remoteDir}` : `Upload files to ${remoteDir}`,
      directory: folder,
      multiple: !folder,
    });
    if (picked === null) return;
    const jobs = (Array.isArray(picked) ? picked : [picked]).map(
      (local): Job => ({
        arrow: "↑",
        name: baseName(local),
        title: `${local} → ${remoteDir}`,
        run: (id, overwrite, onProgress) =>
          uploadPath(id, this.session, this.alias, local, remoteDir, overwrite, onProgress, this.configPath),
      }),
    );
    if ((await this.runJobs(jobs)) && this.path === remoteDir) void this.load(remoteDir);
  }

  /** The menu for item `index`, or for the folder itself when there is none. */
  private openMenu(index: number, x: number, y: number): void {
    const entry = this.entries[index];
    const noFolder = this.path === null;
    const items: MenuItem[] = entry
      ? [
          {
            label: "Download…",
            title: "Download to a local folder",
            disabled: entry.kind === "other",
            run: () => void this.download(entry),
          },
          { label: "Delete…", title: "Delete permanently from the host", danger: true, run: () => void this.delete(entry) },
        ]
      : [
          { label: "Upload files…", title: "Upload files into this folder", disabled: noFolder, run: () => void this.upload(false) },
          { label: "Upload folder…", title: "Upload a folder into this folder", disabled: noFolder, run: () => void this.upload(true) },
        ];
    showMenu(x, y, items, this.list);
  }

  /** Context-menu key or Shift+F10: the menu for the selected item, beside it. */
  private openMenuForKeyboard(): void {
    const item = this.list.querySelector<HTMLElement>('[aria-selected="true"]');
    const rect = (item ?? this.list).getBoundingClientRect();
    this.openMenu(item ? this.selected : -1, rect.left + 16, item ? rect.bottom : rect.top + 4);
  }

  private async delete(entry: Entry): Promise<void> {
    const remoteDir = this.path;
    if (remoteDir === null) return;
    if (!(await confirmDelete(deleteWarning(this.alias, remoteDir, entry)))) return;
    const generation = ++this.generation;
    this.setBusy(true);
    this.showStatus(`Deleting ${joinPath(remoteDir, entry.name)}…`);
    try {
      await deleteRemote(this.session, this.alias, remoteDir, entry.name, this.configPath);
    } catch (e) {
      if (generation !== this.generation) return;
      this.setBusy(false);
      const { message, hint } = describeSftpError(e as SftpError);
      // A folder stops at the first item it cannot delete; what came before is gone.
      const partly = entry.kind === "dir" ? "Part of the folder may be deleted; Refresh to see what is left." : null;
      const lines = [`Not deleted: ${message}`, hint, partly].filter((line) => line !== null);
      this.showStatus(lines.join("\n"), "error");
      return;
    }
    if (generation === this.generation && this.path === remoteDir) void this.load(remoteDir);
  }

  private async download(entry: Entry): Promise<void> {
    const remoteDir = this.path;
    if (!entry || entry.kind === "other" || remoteDir === null) return;
    const localDir = await open({
      title: `Download “${entry.name}” to`,
      directory: true,
      defaultPath: lastDownloadDir,
    });
    if (typeof localDir !== "string") return;
    lastDownloadDir = localDir;
    await this.runJobs([
      {
        arrow: "↓",
        name: entry.name,
        title: `${joinPath(remoteDir, entry.name)} → ${localDir}`,
        run: (id, overwrite, onProgress) =>
          downloadEntry(id, this.session, this.alias, remoteDir, entry.name, localDir, overwrite, onProgress, this.configPath),
      },
    ]);
  }

  /** Runs jobs one after another; true if any completed. */
  private async runJobs(jobs: Job[]): Promise<boolean> {
    const rows = jobs.map((job) => new TransferRow(job));
    this.transfers.append(...rows.map((r) => r.element));
    rows[0]?.element.scrollIntoView({ block: "nearest" });
    let anyDone = false;
    let stopped = false;
    for (const [i, job] of jobs.entries()) {
      const row = rows[i];
      if (stopped) {
        row.finish("cancelled", "Cancelled");
        continue;
      }
      const id = nextTransferId++;
      row.start(() => void cancelTransfer(id));
      try {
        const outcome = await withConflictCheck(
          (overwrite) => job.run(id, overwrite, (progress) => row.progress(progress)),
          askConflict,
        );
        if (outcome === "done") {
          anyDone = true;
          row.finish("done", "Done");
        } else if (outcome === "skipped") {
          row.finish("skipped", "Skipped: already exists");
        } else {
          stopped = true;
          row.finish("cancelled", "Cancelled");
        }
      } catch (e) {
        const error = e as SftpError;
        if (error.kind === "cancelled") {
          row.finish("cancelled", "Cancelled", error.message);
        } else {
          const { message, hint } = describeSftpError(error);
          row.finish("failed", `Failed: ${message}`, hint ? `${message}\n${hint}` : message);
        }
      }
    }
    return anyDone;
  }

  private select(index: number): void {
    this.selected = index;
    this.render();
  }

  private render(): void {
    this.list.replaceChildren(
      ...this.entries.map((entry, i) => {
        const item = document.createElement("li");
        item.role = "option";
        item.className = "file";
        item.ariaSelected = String(i === this.selected);
        item.dataset.index = String(i);
        item.title = `${entry.name}\n${entry.modified}`;
        const icon = document.createElement("span");
        icon.className = "file-icon";
        icon.textContent = icons[entry.kind];
        const name = document.createElement("span");
        name.className = "file-name";
        name.textContent = entry.name;
        const size = document.createElement("span");
        size.className = "file-size";
        size.textContent = entry.kind === "file" ? formatSize(entry.size) : "";
        item.append(icon, name, size);
        item.addEventListener("click", () => this.select(i));
        item.addEventListener("dblclick", () => this.open(i));
        return item;
      }),
    );
    this.list.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
  }

  private handleListKey(e: KeyboardEvent): void {
    if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) {
      e.preventDefault();
      this.openMenuForKeyboard();
      return;
    }
    switch (e.key) {
      case "ArrowDown":
      case "ArrowUp":
        e.preventDefault();
        this.select(moveSelection(this.selected, e.key === "ArrowDown" ? 1 : -1, this.entries.length));
        break;
      case "Enter":
        e.preventDefault();
        this.open(this.selected);
        break;
      case "Backspace":
        e.preventDefault();
        this.up();
        break;
    }
  }

  private setBusy(busy: boolean): void {
    this.element.classList.toggle("busy", busy);
    this.refreshButton.disabled = busy;
  }

  private showStatus(text: string, tone?: "warning" | "error"): void {
    this.status.textContent = text;
    this.status.title = "";
    if (tone) this.status.dataset.tone = tone;
    else delete this.status.dataset.tone;
  }
}
