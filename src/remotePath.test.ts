import { describe, expect, it } from "vitest";
import type { Entry } from "./api";
import { deleteWarning, describeSftpError, formatSize, joinPath, parentPath, sortEntries } from "./remotePath";

describe("joinPath", () => {
  it("adds one separator", () => {
    expect(joinPath("/home/me", "my file.txt")).toBe("/home/me/my file.txt");
    expect(joinPath("/", "etc")).toBe("/etc");
  });
});

describe("parentPath", () => {
  it("drops the last component", () => {
    expect(parentPath("/home/me/máy chủ")).toBe("/home/me");
    expect(parentPath("/home/me/")).toBe("/home");
    expect(parentPath("/home")).toBe("/");
  });

  it("keeps the root", () => {
    expect(parentPath("/")).toBe("/");
  });
});

describe("formatSize", () => {
  it("uses binary units", () => {
    expect(formatSize(0)).toBe("0 B");
    expect(formatSize(1023)).toBe("1023 B");
    expect(formatSize(1536)).toBe("1.5 KiB");
    expect(formatSize(5 * 1024 ** 3)).toBe("5.0 GiB");
  });
});

describe("sortEntries", () => {
  const entry = (name: string, kind: Entry["kind"]): Entry => ({ name, kind, size: 0, modified: "" });

  it("lists folders first, then names naturally", () => {
    const sorted = sortEntries([entry("b10", "file"), entry("Zed", "dir"), entry("b9", "file"), entry("app", "dir")]);
    expect(sorted.map((e) => e.name)).toEqual(["app", "Zed", "b9", "b10"]);
  });
});

describe("describeSftpError", () => {
  it("adds guidance for errors the user can act on", () => {
    expect(describeSftpError({ kind: "hostKey", message: "Host key verification failed." }).hint).toMatch(/terminal/);
    expect(describeSftpError({ kind: "auth", message: "Permission denied (publickey)." }).hint).toMatch(/password/);
  });

  it("shows remote errors as they are", () => {
    expect(describeSftpError({ kind: "remote", message: "stat remote: No such file or directory" })).toEqual({
      message: "stat remote: No such file or directory",
      hint: null,
    });
  });
});

describe("deleteWarning", () => {
  const entry = (kind: Entry["kind"], name: string): Entry => ({ name, kind, size: 0, modified: "" });

  it("says a folder goes with its contents and cannot be restored", () => {
    const text = deleteWarning("web", "/srv", entry("dir", "my site"));
    expect(text).toContain("folder “my site” from web");
    expect(text).toContain("/srv/my site");
    expect(text).toContain("Everything inside it is deleted too.");
    expect(text).toContain("cannot be undone");
  });

  it("says only a link itself is deleted", () => {
    const text = deleteWarning("web", "/", entry("link", "current"));
    expect(text).toContain("link “current”");
    expect(text).toContain("/current");
    expect(text).toContain("not what it points to");
  });

  it("describes a file", () => {
    const text = deleteWarning("web", "/srv", entry("file", "a.txt"));
    expect(text).toContain("file “a.txt”");
    expect(text).not.toContain("Everything inside");
    expect(text).toContain("cannot be undone");
  });
});
