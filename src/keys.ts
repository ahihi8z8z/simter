export type ClipboardAction = "copy" | "paste" | null;

type Key = Pick<KeyboardEvent, "type" | "key" | "ctrlKey" | "shiftKey" | "altKey" | "metaKey">;

/**
 * Which keydowns the app takes from the terminal for the clipboard. Ctrl+C
 * copies only while text is selected; otherwise it is the interrupt (^C) and
 * goes to the remote side. Every other key, including F1-F12, goes to the
 * terminal.
 */
export function clipboardAction(e: Key, hasSelection: boolean): ClipboardAction {
  if (e.type !== "keydown" || !e.ctrlKey || e.altKey || e.metaKey) return null;
  const key = e.key.toLowerCase();
  if (key === "c" && (e.shiftKey || hasSelection)) return "copy";
  if (key === "v") return "paste";
  return null;
}
