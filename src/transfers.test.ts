import { describe, expect, it, vi } from "vitest";
import { baseName, withConflictCheck, type Choice } from "./transfers";

const exists = { kind: "exists", message: "“a.txt” already exists in /srv" };

describe("withConflictCheck", () => {
  it("does not ask when the destination is free", async () => {
    const attempt = vi.fn(async () => {});
    const ask = vi.fn(async (): Promise<Choice> => "overwrite");
    expect(await withConflictCheck(attempt, ask)).toBe("done");
    expect(attempt).toHaveBeenCalledExactlyOnceWith(false);
    expect(ask).not.toHaveBeenCalled();
  });

  it("overwrites only after the user chooses to", async () => {
    const attempt = vi.fn(async (overwrite: boolean) => {
      if (!overwrite) throw exists;
    });
    const ask = vi.fn(async (): Promise<Choice> => "overwrite");
    expect(await withConflictCheck(attempt, ask)).toBe("done");
    expect(ask).toHaveBeenCalledWith(exists.message);
    expect(attempt.mock.calls).toEqual([[false], [true]]);
  });

  it.each([
    ["skip", "skipped"],
    ["cancel", "cancelled"],
  ] as const)("never overwrites on %s", async (choice, outcome) => {
    const attempt = vi.fn(async (overwrite: boolean) => {
      if (!overwrite) throw exists;
    });
    expect(await withConflictCheck(attempt, async () => choice)).toBe(outcome);
    expect(attempt).toHaveBeenCalledExactlyOnceWith(false);
  });

  it("passes other errors through", async () => {
    const failure = { kind: "remote", message: "Permission denied" };
    const attempt = async () => {
      throw failure;
    };
    await expect(withConflictCheck(attempt, async () => "overwrite")).rejects.toBe(failure);
  });
});

describe("baseName", () => {
  it("handles Windows and remote paths", () => {
    expect(baseName(String.raw`C:\Users\me\máy chủ.txt`)).toBe("máy chủ.txt");
    expect(baseName("D:\\data\\folder\\")).toBe("folder");
    expect(baseName("/srv/my dir/")).toBe("my dir");
  });
});
