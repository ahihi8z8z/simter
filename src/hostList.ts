// Pure host-list logic, kept free of DOM and Tauri so it can be unit tested.

/** True if every character of `needle` appears in `hay` in order. */
function isSubsequence(needle: string, hay: string): boolean {
  let i = 0;
  for (const c of hay) {
    if (c === needle[i]) i++;
    if (i === needle.length) return true;
  }
  return needle.length === 0;
}

/**
 * Case-insensitive alias filter. Prefix matches rank first, then substring
 * matches, then in-order (fuzzy) matches. Order within a rank is preserved.
 */
export function filterHosts<T extends { alias: string }>(hosts: T[], query: string): T[] {
  const q = query.trim().toLowerCase();
  if (!q) return hosts;
  const ranked: [number, T][] = [];
  for (const host of hosts) {
    const alias = host.alias.toLowerCase();
    const at = alias.indexOf(q);
    if (at === 0) ranked.push([0, host]);
    else if (at > 0) ranked.push([1, host]);
    else if (isSubsequence(q, alias)) ranked.push([2, host]);
  }
  // Array.prototype.sort is stable.
  return ranked.sort((a, b) => a[0] - b[0]).map(([, host]) => host);
}

/**
 * Recently opened hosts first, most recent first; the rest keep their config order.
 * `recent` is most recent first and may name hosts no longer in the config.
 */
export function orderByRecent<T extends { alias: string }>(hosts: T[], recent: string[]): T[] {
  const rank = new Map(recent.map((alias, i) => [alias, i]));
  const at = (h: T) => rank.get(h.alias) ?? recent.length;
  // Array.prototype.sort is stable, so unranked hosts keep their order.
  return [...hosts].sort((a, b) => at(a) - at(b));
}

/** Moves a selection index by `delta`, clamped to the list; -1 when empty. */
export function moveSelection(index: number, delta: number, length: number): number {
  if (length === 0) return -1;
  if (index < 0) return delta > 0 ? 0 : length - 1;
  return Math.min(length - 1, Math.max(0, index + delta));
}

/** Index of `alias` in `hosts`, else 0 for a non-empty list, else -1. */
export function reselect(hosts: { alias: string }[], alias: string | undefined): number {
  const found = alias === undefined ? -1 : hosts.findIndex((h) => h.alias === alias);
  if (found >= 0) return found;
  return hosts.length > 0 ? 0 : -1;
}

/**
 * Where "Open config" should point: the host's single declaration, else the
 * main config (no host, or an alias declared in several places).
 */
export function configTarget(
  host: { locations: { file: string; line: number }[] } | undefined,
  configPath: string,
): { file: string; line: number | null } {
  if (host?.locations.length === 1) return { ...host.locations[0] };
  return { file: configPath, line: null };
}

/**
 * The `-F` path to use after the user picks `picked`: undefined (no `-F`)
 * when it is the default config, so ssh keeps reading the system-wide config.
 */
export function chooseConfig(picked: string, defaultPath: string): string | undefined {
  const norm = (p: string) => p.replaceAll("/", "\\").toLowerCase();
  return defaultPath !== "" && norm(picked) === norm(defaultPath) ? undefined : picked;
}

export interface Details {
  user: string | null;
  hostname: string | null;
  port: number | null;
}

/** `user@hostname:port`, leaving out unresolved parts; IPv6 addresses are bracketed. */
export function formatDetails(d: Details): string {
  let host = d.hostname ?? "";
  if (host.includes(":") && d.port !== null) host = `[${host}]`;
  const user = d.user ? `${d.user}@` : "";
  const port = d.port !== null ? `:${d.port}` : "";
  return `${user}${host}${port}`;
}
