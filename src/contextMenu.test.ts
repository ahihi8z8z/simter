import { describe, expect, it } from "vitest";
import { menuPosition } from "./contextMenu";

describe("menuPosition", () => {
  it("opens at the pointer when it fits", () => {
    expect(menuPosition(10, 20, 100, 50, 800, 600)).toEqual({ left: 10, top: 20 });
  });

  it("opens up and to the left near the bottom-right corner", () => {
    expect(menuPosition(790, 590, 100, 50, 800, 600)).toEqual({ left: 690, top: 540 });
  });

  it("stays inside a viewport smaller than the flipped menu", () => {
    expect(menuPosition(50, 30, 100, 50, 120, 60)).toEqual({ left: 0, top: 0 });
  });
});
