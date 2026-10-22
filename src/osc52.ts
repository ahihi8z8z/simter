/**
 * The text of an OSC 52 clipboard write (`ESC ] 52 ; Pc ; Pd BEL`), which tmux and Byobu send
 * when text is selected in them. `data` is what follows `52;`. Returns null for a clipboard
 * query (`?`), which is never answered so the host cannot read the local clipboard, and for
 * anything that is not valid base64 UTF-8.
 */
export function osc52Text(data: string): string | null {
  const separator = data.indexOf(";");
  if (separator < 0) return null;
  const payload = data.slice(separator + 1);
  if (payload === "" || payload === "?") return null;
  let binary: string;
  try {
    binary = atob(payload);
  } catch {
    return null;
  }
  const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}
