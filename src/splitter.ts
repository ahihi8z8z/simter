const MIN_PANE = 180;
const KEY_STEP = 24;

/** Clamps a pane size so both it and its neighbour keep `min` pixels. */
export function clampWidth(width: number, container: number, min = MIN_PANE): number {
  return Math.round(Math.max(min, Math.min(width, container - min)));
}

/**
 * A divider that resizes `pane`, the element after it: to its right for a
 * "vertical" divider, below it for a "horizontal" one. Resizes by dragging,
 * or with the arrow keys while the divider has focus. `onResized` gets the
 * new size once a drag or key press ends.
 */
export function createSplitter(
  pane: HTMLElement,
  onResized?: (size: number) => void,
  orientation: "vertical" | "horizontal" = "vertical",
  min = MIN_PANE,
): HTMLDivElement {
  const vertical = orientation === "vertical";
  const splitter = document.createElement("div");
  splitter.className = `splitter ${orientation}`;
  splitter.role = "separator";
  splitter.ariaOrientation = orientation;
  splitter.tabIndex = 0;
  splitter.title = "Drag to resize";

  const size = () => (vertical ? pane.offsetWidth : pane.offsetHeight);
  const setSize = (value: number) => {
    const parent = splitter.parentElement;
    const container = (vertical ? parent?.clientWidth : parent?.clientHeight) ?? 0;
    if (container > 0) pane.style.flexBasis = `${clampWidth(value, container, min)}px`;
  };

  splitter.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    splitter.setPointerCapture(e.pointerId);
    splitter.classList.add("dragging");
  });
  splitter.addEventListener("pointermove", (e) => {
    if (!splitter.hasPointerCapture(e.pointerId)) return;
    // Measured from the pane's far edge, so elements after it do not count.
    const box = pane.getBoundingClientRect();
    setSize(
      vertical
        ? box.right - e.clientX - splitter.offsetWidth / 2
        : box.bottom - e.clientY - splitter.offsetHeight / 2,
    );
  });
  const stop = (e: PointerEvent) => {
    if (!splitter.hasPointerCapture(e.pointerId)) return;
    splitter.releasePointerCapture(e.pointerId);
    splitter.classList.remove("dragging");
    onResized?.(size());
  };
  splitter.addEventListener("pointerup", stop);
  splitter.addEventListener("pointercancel", stop);
  splitter.addEventListener("keydown", (e) => {
    const [grow, shrink] = vertical ? ["ArrowLeft", "ArrowRight"] : ["ArrowUp", "ArrowDown"];
    if (e.key !== grow && e.key !== shrink) return;
    e.preventDefault();
    setSize(size() + (e.key === grow ? KEY_STEP : -KEY_STEP));
    onResized?.(size());
  });
  return splitter;
}
