import { message, open } from "@tauri-apps/plugin-dialog";
import {
  endAskpassSession,
  hostDetails,
  listHosts,
  loadPrefs,
  onAskpass,
  openConfig,
  recordRecentHost,
  saveConfigPathPref,
  saveLayoutPref,
  type Host,
  type HostList,
  type Issue,
  type Layout,
} from "./api";
import { showQuestion } from "./askpassDialog";
import {
  chooseConfig,
  configTarget,
  filterHosts,
  formatDetails,
  moveSelection,
  orderByRecent,
  reselect,
} from "./hostList";
import type { State } from "./connection";
import { FilePane } from "./filePane";
import { createSplitter } from "./splitter";
import { nextAfterClose } from "./tabs";
import { TerminalView } from "./terminalView";

function el<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing #${id}`);
  return node as T;
}

const search = el<HTMLInputElement>("search");
const refreshButton = el<HTMLButtonElement>("refresh");
const connectButton = el<HTMLButtonElement>("connect");
const hostList = el<HTMLUListElement>("host-list");
const issuesBox = el<HTMLDivElement>("issues");
const issuesToggle = el<HTMLButtonElement>("issues-toggle");
const issuesList = el<HTMLUListElement>("issues-list");
const configPathButton = el<HTMLButtonElement>("config-path");
const openConfigButton = el<HTMLButtonElement>("open-config");
const placeholder = el<HTMLParagraphElement>("placeholder");
const sessionBar = el<HTMLDivElement>("session-bar");
const sessionTitle = el<HTMLSpanElement>("session-title");
const sessionStatus = el<HTMLSpanElement>("session-status");
const reconnectButton = el<HTMLButtonElement>("reconnect");
const tabBar = el<HTMLDivElement>("tabs");
const terminalHost = el<HTMLDivElement>("terminal-host");
const sidebar = el<HTMLElement>("sidebar");
const hostsView = el<HTMLElement>("hosts-view");
const filesView = el<HTMLElement>("files-view");
const filesHost = el<HTMLDivElement>("files-host");
const filesTitle = el<HTMLSpanElement>("files-title");
const filesEmpty = el<HTMLParagraphElement>("files-empty");
const sidebarToggle = el<HTMLButtonElement>("sidebar-toggle");

let all: Host[] = [];
let visible: Host[] = [];
let selected = -1;
let loadError: string | null = null;
// The SSH config file in use, as reported by the last refresh.
let configPath = "";
// A user-chosen config, passed to ssh and sftp as -F; undefined means the default.
let customConfig: string | undefined;
// %USERPROFILE%\.ssh\config as reported by the backend, once known.
let defaultConfig = "";
// Host aliases opened in the app, most recent first; listed first.
let recentHosts: string[] = [];
// Side bar size and collapsed state, saved in prefs.
let layout: Layout = {};
let prefsErrorShown = false;

/** Reports a preferences problem once per session; the app keeps working. */
function prefsError(e: unknown): void {
  if (prefsErrorShown) return;
  prefsErrorShown = true;
  void message(`${e}\n\nSettings will not be saved until this is fixed.`, {
    title: "Preferences problem",
    kind: "warning",
  });
}

// `ssh -G` results per alias, resolved lazily for the selected host and
// cleared on Refresh.
type Resolved = { text: string } | { error: string };
const details = new Map<string, Resolved | "pending">();
let detailsTimer: number | undefined;
let refreshGeneration = 0;

function requestDetails(): void {
  window.clearTimeout(detailsTimer);
  const alias = visible[selected]?.alias;
  if (alias === undefined || details.has(alias)) return;
  // Debounced so holding an arrow key does not start an ssh per host.
  detailsTimer = window.setTimeout(() => {
    details.set(alias, "pending");
    const generation = refreshGeneration;
    hostDetails(alias, customConfig)
      .then(
        (d): Resolved => ({ text: formatDetails(d) }),
        (e): Resolved => ({ error: String(e) }),
      )
      .then((result) => {
        if (generation !== refreshGeneration) return;
        details.set(alias, result);
        render();
      });
  }, 150);
}

function render(): void {
  // Without a search, recent hosts are at the top; a line separates them from the rest.
  const recentCount = search.value.trim() ? 0 : visible.filter((h) => recentHosts.includes(h.alias)).length;
  hostList.replaceChildren(
    ...visible.map((host, i) => {
      const item = document.createElement("li");
      item.role = "option";
      item.className = "host";
      if (i === recentCount - 1 && recentCount < visible.length) item.classList.add("recent-last");
      item.ariaSelected = String(i === selected);
      const name = document.createElement("span");
      name.textContent = host.alias;
      item.append(name);
      const resolved = details.get(host.alias);
      if (i === selected && resolved !== undefined && resolved !== "pending") {
        const line = document.createElement("span");
        line.className = "details";
        if ("text" in resolved) {
          line.textContent = resolved.text;
          line.title = "Effective settings from ssh -G";
        } else {
          line.textContent = "details unavailable";
          line.title = resolved.error;
          line.classList.add("unavailable");
        }
        item.append(line);
      }
      if (host.conditional) {
        const badge = document.createElement("span");
        badge.className = "badge";
        badge.textContent = "conditional";
        badge.title = "Declared in a file included from a Host/Match block; ssh reads it only when that block matches";
        item.append(badge);
      }
      item.addEventListener("click", () => select(i));
      item.addEventListener("dblclick", () => connect());
      return item;
    }),
  );
  if (visible.length === 0) {
    const empty = document.createElement("li");
    empty.className = "empty";
    empty.textContent = loadError ? "Could not load hosts." : all.length ? "No matching hosts." : "No hosts found.";
    hostList.append(empty);
  }
  connectButton.disabled = selected < 0;
  openConfigButton.disabled = configPath === "";
  requestDetails();
  hostList.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}

function describeIssue(issue: Issue): string {
  const where = issue.line === null ? issue.file : `${issue.file}:${issue.line}`;
  return `${where} — ${issue.message}`;
}

function renderIssues(issues: Issue[]): void {
  const messages = loadError ? [`Host discovery failed: ${loadError}`] : issues.map(describeIssue);
  issuesBox.hidden = messages.length === 0;
  const errors = loadError ? 1 : issues.filter((i) => i.severity === "error").length;
  issuesBox.classList.toggle("has-errors", errors > 0);
  issuesToggle.textContent = `${messages.length} config ${messages.length === 1 ? "problem" : "problems"}`;
  issuesList.replaceChildren(
    ...messages.map((text, i) => {
      const item = document.createElement("li");
      item.className = loadError || issues[i].severity === "error" ? "error" : "warning";
      item.textContent = text;
      return item;
    }),
  );
}

/** The hosts matching the search, recently opened ones first. */
function filtered(): Host[] {
  return filterHosts(orderByRecent(all, recentHosts), search.value);
}

function applyFilter(): void {
  const previous = visible[selected]?.alias;
  visible = filtered();
  selected = search.value.trim() ? (visible.length ? 0 : -1) : reselect(visible, previous);
  render();
}

function select(index: number): void {
  selected = index;
  render();
}

interface Tab {
  view: TerminalView;
  files: FilePane;
  /** The terminal; the file pane lives in the side bar's Files view. */
  body: HTMLDivElement;
  button: HTMLDivElement;
  state: State;
  detail: string;
  /** The -F config the tab was opened with. */
  configPath: string | undefined;
  /** Askpass session: passwords typed for this tab are remembered under it until it closes. */
  session: string;
}

const tabs: Tab[] = [];
let active: Tab | null = null;

function renderSessionBar(): void {
  sessionBar.hidden = active === null;
  placeholder.hidden = active !== null;
  if (!active) return;
  sessionTitle.textContent = active.view.alias;
  sessionStatus.dataset.status = active.state;
  sessionStatus.textContent = active.detail;
  reconnectButton.hidden = active.view.connected;
}

function activate(tab: Tab): void {
  active = tab;
  for (const t of tabs) {
    const isActive = t === tab;
    t.body.hidden = !isActive;
    t.files.element.hidden = !isActive;
    t.button.ariaSelected = String(isActive);
  }
  renderSessionBar();
  renderFilesView();
  tab.view.show();
}

function closeTab(tab: Tab): void {
  const index = tabs.indexOf(tab);
  if (index < 0) return;
  const next = nextAfterClose(index, tabs.length);
  tabs.splice(index, 1);
  tab.view.dispose();
  void endAskpassSession(tab.session);
  tab.body.remove();
  tab.files.element.remove();
  tab.button.remove();
  tabBar.hidden = tabs.length === 0;
  if (active === tab) {
    active = null;
    if (next >= 0) activate(tabs[next]);
    else {
      renderSessionBar();
      renderFilesView();
      search.focus();
    }
  }
}

function createTab(alias: string, configPath: string | undefined): Tab {
  const button = document.createElement("div");
  button.role = "tab";
  button.className = "tab";
  const dot = document.createElement("span");
  dot.className = "tab-state";
  const label = document.createElement("span");
  label.className = "tab-label";
  label.textContent = alias;
  const close = document.createElement("button");
  close.type = "button";
  close.className = "tab-close";
  close.textContent = "×";
  close.title = "Close (ends this ssh process; Byobu sessions keep running on the host)";
  button.append(dot, label, close);

  const session = crypto.randomUUID();
  const files = new FilePane(alias, session, configPath);
  const body = document.createElement("div");
  body.className = "tab-body";
  const tab: Tab = {
    view: new TerminalView(
      alias,
      session,
      (state, detail) => {
        tab.state = state;
        tab.detail = detail;
        button.dataset.state = state;
        button.title = configPath ? `${alias} (${configPath}) — ${detail}` : `${alias} — ${detail}`;
        if (active === tab) renderSessionBar();
      },
      configPath,
    ),
    files,
    body,
    button,
    state: "connecting",
    detail: "",
    configPath,
    session,
  };
  body.append(tab.view.element);
  button.addEventListener("click", () => activate(tab));
  close.addEventListener("click", (e) => {
    e.stopPropagation();
    closeTab(tab);
  });
  return tab;
}

function renderFilesView(): void {
  filesTitle.textContent = active ? ` — ${active.view.alias}` : "";
  filesEmpty.hidden = active !== null;
}

function saveLayout(): void {
  saveLayoutPref(layout).catch(prefsError);
}

const sidebarSplitter = createSplitter(sidebar, (width) => {
  layout.sidebarWidth = width;
  saveLayout();
});
sidebar.before(sidebarSplitter);
const viewSplitter = createSplitter(
  filesView,
  (height) => {
    layout.filesHeight = height;
    saveLayout();
  },
  "horizontal",
  120,
);
filesView.before(viewSplitter);

/** Shows the saved sizes and collapsed state. */
function applyLayout(): void {
  if (layout.sidebarWidth !== undefined) sidebar.style.flexBasis = `${layout.sidebarWidth}px`;
  if (layout.filesHeight !== undefined) filesView.style.flexBasis = `${layout.filesHeight}px`;
  const hidden = !!layout.sidebarHidden;
  sidebar.hidden = hidden;
  sidebarSplitter.hidden = hidden;
  sidebarToggle.ariaPressed = String(!hidden);
  for (const [view, collapsed] of [
    [hostsView, !!layout.hostsCollapsed],
    [filesView, !!layout.filesCollapsed],
  ] as const) {
    view.classList.toggle("collapsed", collapsed);
    view.querySelector(".view-head")?.setAttribute("aria-expanded", String(!collapsed));
  }
  sidebar.classList.toggle("hosts-collapsed", !!layout.hostsCollapsed);
  // Sizing between the views only makes sense while both are open.
  viewSplitter.hidden = !!layout.hostsCollapsed || !!layout.filesCollapsed;
}

sidebarToggle.addEventListener("click", () => {
  layout.sidebarHidden = !layout.sidebarHidden;
  applyLayout();
  saveLayout();
  if (!layout.sidebarHidden && !layout.hostsCollapsed) search.focus();
  else active?.view.show();
});
hostsView.querySelector(".view-head")?.addEventListener("click", () => {
  layout.hostsCollapsed = !layout.hostsCollapsed;
  applyLayout();
  saveLayout();
});
filesView.querySelector(".view-head")?.addEventListener("click", () => {
  layout.filesCollapsed = !layout.filesCollapsed;
  applyLayout();
  saveLayout();
});

/** Focuses the selected host's tab, opening one if it has none. */
function connect(): void {
  const host = visible[selected];
  if (!host) return;
  recordRecentHost(host.alias)
    .then((recent) => {
      recentHosts = recent;
      const previous = visible[selected]?.alias;
      visible = filtered();
      selected = reselect(visible, previous);
      render();
    })
    .catch(prefsError);
  const existing = tabs.find((t) => t.view.alias === host.alias && t.configPath === customConfig);
  if (existing) {
    activate(existing);
    return;
  }
  const tab = createTab(host.alias, customConfig);
  tabs.push(tab);
  tabBar.append(tab.button);
  tabBar.hidden = false;
  terminalHost.append(tab.body);
  filesHost.append(tab.files.element);
  activate(tab);
  void tab.view.connect();
  void tab.files.load();
}

async function refresh(): Promise<void> {
  const previous = visible[selected]?.alias;
  refreshButton.disabled = true;
  refreshGeneration++;
  details.clear();
  try {
    const result: HostList = await listHosts(customConfig);
    all = result.hosts;
    loadError = null;
    configPath = result.configPath;
    defaultConfig = result.defaultConfigPath;
    configPathButton.textContent = result.configPath;
    configPathButton.title = `${result.configPath}\nClick to use a different SSH config file (passed to ssh and sftp as -F). Pick the default file to go back.`;
    renderIssues(result.issues);
  } catch (e) {
    all = [];
    loadError = String(e);
    renderIssues([]);
  } finally {
    refreshButton.disabled = false;
  }
  visible = filtered();
  selected = reselect(visible, previous);
  render();
}

search.addEventListener("input", applyFilter);
search.addEventListener("keydown", (e) => {
  switch (e.key) {
    case "ArrowDown":
    case "ArrowUp":
      e.preventDefault();
      select(moveSelection(selected, e.key === "ArrowDown" ? 1 : -1, visible.length));
      break;
    case "Enter":
      e.preventDefault();
      connect();
      break;
    case "Escape":
      search.value = "";
      applyFilter();
      break;
  }
});
refreshButton.addEventListener("click", () => void refresh());
connectButton.addEventListener("click", connect);
configPathButton.addEventListener("click", async () => {
  const picked = await open({ title: "Choose an SSH config file", defaultPath: configPath || undefined });
  if (typeof picked !== "string") return;
  customConfig = chooseConfig(picked, defaultConfig);
  saveConfigPathPref(customConfig).catch(prefsError);
  await refresh();
});
openConfigButton.addEventListener("click", () => {
  const { file, line } = configTarget(visible[selected], configPath);
  openConfig(file, line).catch((e) => void message(String(e), { title: "Could not open the config", kind: "error" }));
});
reconnectButton.addEventListener("click", () => void active?.view.connect());
issuesToggle.addEventListener("click", () => {
  issuesList.hidden = !issuesList.hidden;
});

async function start(): Promise<void> {
  await onAskpass((question) => void showQuestion(question));
  try {
    const prefs = await loadPrefs();
    customConfig = prefs.configPath;
    layout = prefs.layout ?? {};
    recentHosts = prefs.recentHosts ?? [];
  } catch (e) {
    prefsError(e);
  }
  applyLayout();
  await refresh();
}

void start();
