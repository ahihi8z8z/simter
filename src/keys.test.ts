import { describe, expect, it } from "vitest";
import { clipboardAction } from "./keys";

function key(k: string, mods: { ctrl?: boolean; shift?: boolean; alt?: boolean } = {}, type = "keydown") {
  return { type, key: k, ctrlKey: !!mods.ctrl, shiftKey: !!mods.shift, altKey: !!mods.alt, metaKey: false };
}

describe("clipboardAction", () => {
  it("copies with Ctrl+Shift+C, or Ctrl+C when text is selected", () => {
    expect(clipboardAction(key("C", { ctrl: true, shift: true }), false)).toBe("copy");
    expect(clipboardAction(key("c", { ctrl: true }), true)).toBe("copy");
  });

  it("leaves Ctrl+C without a selection to the terminal as an interrupt", () => {
    expect(clipboardAction(key("c", { ctrl: true }), false)).toBeNull();
  });

  it("pastes with Ctrl+V and Ctrl+Shift+V", () => {
    expect(clipboardAction(key("v", { ctrl: true }), false)).toBe("paste");
    expect(clipboardAction(key("V", { ctrl: true, shift: true }), false)).toBe("paste");
  });

  it("never claims function keys or other shortcuts", () => {
    for (const k of ["F2", "F3", "F5", "F7", "F8", "F9", "F12"]) {
      expect(clipboardAction(key(k), false)).toBeNull();
      expect(clipboardAction(key(k, { shift: true }), false)).toBeNull();
      expect(clipboardAction(key(k, { ctrl: true }), false)).toBeNull();
    }
    expect(clipboardAction(key("f", { ctrl: true }), false)).toBeNull();
    expect(clipboardAction(key("v", { ctrl: true, alt: true }), false)).toBeNull();
  });

  it("ignores key releases", () => {
    expect(clipboardAction(key("v", { ctrl: true }, "keyup"), false)).toBeNull();
  });
});
