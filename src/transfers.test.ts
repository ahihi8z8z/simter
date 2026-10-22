import { describe, expect, it, vi } from "vitest";
import { baseName, describeProgress, withConflictCheck, type Choice } from "./transfers";

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

describe("describeProgress", () => {
  it("shows a percentage once the total is known", () => {
    expect(describeProgress({ done: 512 * 1024, total: 2048 * 1024 })).toEqual({
      fraction: 0.25,
      text: "25% · 512.0 KiB of 2.0 MiB",
    });
    expect(describeProgress({ done: 0, total: 10 }).text).toBe("0% · 0 B of 10 B");
  });

  it("never goes past 100%", () => {
    expect(describeProgress({ done: 150, total: 100 })).toEqual({ fraction: 1, text: "100% · 100 B of 100 B" });
  });

  it("shows bytes alone while the total is unknown", () => {
    expect(describeProgress({ done: 0, total: null })).toEqual({ fraction: null, text: "Running…" });
    expect(describeProgress({ done: 2048, total: null })).toEqual({ fraction: null, text: "2.0 KiB" });
    expect(describeProgress({ done: 0, total: 0 }).fraction).toBeNull();
  });
});
