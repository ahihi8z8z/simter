export interface MenuItem {
  label: string;
  title?: string;
  disabled?: boolean;
  danger?: boolean;
  run(): void;
}

/** Top-left corner for a `width`×`height` menu opened at (x, y), kept inside the viewport. */
export function menuPosition(
  x: number,
  y: number,
  width: number,
  height: number,
  viewWidth: number,
  viewHeight: number,
): { left: number; top: number } {
  // Open up or to the left when there is no room below or to the right.
  const left = x + width <= viewWidth ? x : x - width;
  const top = y + height <= viewHeight ? y : y - height;
  return {
    left: Math.max(0, Math.min(left, viewWidth - width)),
    top: Math.max(0, Math.min(top, viewHeight - height)),
  };
}

let current: { close(): void } | null = null;

export function isMenuOpen(): boolean {
  return current !== null;
}

/**
 * Shows a menu at (x, y), replacing any open one. It closes on a choice, Escape, a click
 * elsewhere, scrolling, resizing or the window losing focus; focus then returns to `returnFocus`.
 */
export function showMenu(x: number, y: number, items: MenuItem[], returnFocus?: { focus(): void }): void {
  current?.close();
  const menu = document.createElement("div");
  menu.className = "context-menu";
  menu.role = "menu";
  const buttons = items.map((item) => {
    const b = document.createElement("button");
    b.type = "button";
    b.role = "menuitem";
    b.textContent = item.label;
    if (item.title) b.title = item.title;
    b.disabled = item.disabled ?? false;
    if (item.danger) b.classList.add("danger");
    b.addEventListener("click", () => {
      close();
      item.run();
    });
    return b;
  });
  menu.append(...buttons);
  document.body.append(menu);
  const { left, top } = menuPosition(x, y, menu.offsetWidth, menu.offsetHeight, innerWidth, innerHeight);
  menu.style.left = `${left}px`;
  menu.style.top = `${top}px`;

  const enabled = buttons.filter((b) => !b.disabled);
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const at = enabled.indexOf(document.activeElement as HTMLButtonElement);
      const step = e.key === "ArrowDown" ? 1 : -1;
      enabled[(at + step + enabled.length) % enabled.length]?.focus();
    } else if (e.key === "Tab") {
      e.preventDefault();
    }
  };
  const onPointer = (e: PointerEvent) => {
    if (!menu.contains(e.target as Node)) close();
  };
  const onDismiss = () => close();
  menu.addEventListener("keydown", onKey);
  // Capture, so a click elsewhere closes the menu before it does anything else.
  addEventListener("pointerdown", onPointer, true);
  addEventListener("scroll", onDismiss, true);
  addEventListener("resize", onDismiss);
  addEventListener("blur", onDismiss);

  function close(): void {
    if (current !== handle) return;
    current = null;
    removeEventListener("pointerdown", onPointer, true);
    removeEventListener("scroll", onDismiss, true);
    removeEventListener("resize", onDismiss);
    removeEventListener("blur", onDismiss);
    const hadFocus = menu.contains(document.activeElement);
    menu.remove();
    if (hadFocus) returnFocus?.focus();
  }
  const handle = { close };
  current = handle;
  enabled[0]?.focus();
}
