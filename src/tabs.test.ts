import { describe, expect, it } from "vitest";
import { nextAfterClose } from "./tabs";

describe("nextAfterClose", () => {
  it("selects the tab that moved into the closed slot", () => {
    expect(nextAfterClose(0, 3)).toBe(0);
    expect(nextAfterClose(1, 3)).toBe(1);
  });

  it("falls back to the previous tab when the last one closes", () => {
    expect(nextAfterClose(2, 3)).toBe(1);
  });

  it("returns -1 when no tabs remain", () => {
    expect(nextAfterClose(0, 1)).toBe(-1);
  });
});
