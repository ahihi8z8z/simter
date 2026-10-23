import { describe, expect, it, vi } from "vitest";
import { baseName, describeProgress, SpeedMeter, withConflictCheck, type Choice } from "./transfers";

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
  const file = (bytes: number) => ({ bytes, files: null });
  const folder = (bytes: number, files: number | null) => ({ bytes, files });

  it("shows a file's percentage, size and speed", () => {
    expect(describeProgress({ done: file(512 * 1024), total: file(2048 * 1024) }, 1.5 * 1024 * 1024)).toEqual({
      fraction: 0.25,
      text: "25% · 512.0 KiB of 2.0 MiB · 1.5 MiB/s",
    });
    expect(describeProgress({ done: file(0), total: file(10) }).text).toBe("0% · 0 B of 10 B");
  });

  it("counts a folder's files, not the one still being written", () => {
    const total = folder(1000, 10);
    expect(describeProgress({ done: folder(0, null), total }).text).toBe("0% · 0 B of 1000 B · 0/10 files");
    expect(describeProgress({ done: folder(450, 4), total }, 100).text).toBe(
      "45% · 450 B of 1000 B · 3/10 files · 100 B/s",
    );
    expect(describeProgress({ done: folder(1000, 10), total }).text).toBe("100% · 1000 B of 1000 B · 10/10 files");
  });

  it("never goes past 100%", () => {
    expect(describeProgress({ done: folder(150, 7), total: folder(100, 5) })).toEqual({
      fraction: 1,
      text: "100% · 100 B of 100 B · 5/5 files",
    });
  });

  it("shows bytes and speed alone while the total is unknown", () => {
    expect(describeProgress({ done: file(0), total: null })).toEqual({ fraction: null, text: "Running…" });
    expect(describeProgress({ done: file(2048), total: null }, 1024)).toEqual({
      fraction: null,
      text: "2.0 KiB · 1.0 KiB/s",
    });
    expect(describeProgress({ done: folder(0, 0), total: folder(0, 0) }).fraction).toBeNull();
  });
});

describe("SpeedMeter", () => {
  it("averages over the last few seconds", () => {
    const meter = new SpeedMeter(1000);
    expect(meter.sample(0, 0)).toBeNull();
    expect(meter.sample(500, 500)).toBe(1000);
    expect(meter.sample(1000, 1000)).toBe(1000);
    // Slower now: the first sample has left the window.
    expect(meter.sample(1100, 1500)).toBe(600);
    expect(meter.sample(1200, 2000)).toBe(200);
  });

  it("starts over when the copy restarts", () => {
    const meter = new SpeedMeter(1000);
    meter.sample(0, 0);
    meter.sample(5000, 500);
    expect(meter.sample(0, 1000)).toBeNull();
    expect(meter.sample(300, 1500)).toBe(600);
  });
});
