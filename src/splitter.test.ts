import { describe, expect, it } from "vitest";
import { clampWidth } from "./splitter";

describe("clampWidth", () => {
  it("keeps the requested width when both sides fit", () => {
    expect(clampWidth(400, 1200)).toBe(400);
  });

  it("leaves the minimum to each side", () => {
    expect(clampWidth(50, 1200)).toBe(180);
    expect(clampWidth(1150, 1200)).toBe(1020);
  });
});
