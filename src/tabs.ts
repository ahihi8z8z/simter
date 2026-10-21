/**
 * Which tab to show after closing the one at `index` out of `length` tabs:
 * the next one, else the previous. Returns an index into the remaining tabs,
 * or -1 if none remain.
 */
export function nextAfterClose(index: number, length: number): number {
  if (length <= 1) return -1;
  return index < length - 1 ? index : index - 1;
}
