import { describe, expect, it } from "vitest";
import {
  chooseConfig,
  configTarget,
  filterHosts,
  formatDetails,
  moveSelection,
  orderByRecent,
  reselect,
} from "./hostList";

const hosts = ["web-prod", "db-prod", "Staging-Web", "bastion", "máy-chủ"].map((alias) => ({ alias }));
const aliases = (list: { alias: string }[]) => list.map((h) => h.alias);

describe("filterHosts", () => {
  it("returns everything for an empty or blank query", () => {
    expect(filterHosts(hosts, "")).toBe(hosts);
    expect(filterHosts(hosts, "   ")).toBe(hosts);
  });

  it("is case-insensitive and ranks prefix before substring before fuzzy", () => {
    expect(aliases(filterHosts(hosts, "WEB"))).toEqual(["web-prod", "Staging-Web"]);
    expect(aliases(filterHosts(hosts, "prod"))).toEqual(["web-prod", "db-prod"]);
    expect(aliases(filterHosts(hosts, "bst"))).toEqual(["bastion"]);
  });

  it("matches Unicode aliases", () => {
    expect(aliases(filterHosts(hosts, "MÁY"))).toEqual(["máy-chủ"]);
  });

  it("returns nothing when no alias matches", () => {
    expect(filterHosts(hosts, "zzz")).toEqual([]);
  });
});

describe("orderByRecent", () => {
  it("puts recently opened hosts first, most recent first, and keeps the rest in order", () => {
    expect(aliases(orderByRecent(hosts, ["bastion", "gone", "db-prod"]))).toEqual([
      "bastion",
      "db-prod",
      "web-prod",
      "Staging-Web",
      "máy-chủ",
    ]);
  });

  it("leaves the order alone with no recent hosts", () => {
    expect(aliases(orderByRecent(hosts, []))).toEqual(aliases(hosts));
  });

  it("keeps recent hosts first within a search rank", () => {
    const ordered = orderByRecent(hosts, ["db-prod"]);
    expect(aliases(filterHosts(ordered, "prod"))).toEqual(["db-prod", "web-prod"]);
  });
});

describe("moveSelection", () => {
  it("clamps to the list bounds", () => {
    expect(moveSelection(0, -1, 3)).toBe(0);
    expect(moveSelection(2, 1, 3)).toBe(2);
    expect(moveSelection(1, 1, 3)).toBe(2);
  });

  it("enters the list from no selection", () => {
    expect(moveSelection(-1, 1, 3)).toBe(0);
    expect(moveSelection(-1, -1, 3)).toBe(2);
    expect(moveSelection(0, 1, 0)).toBe(-1);
  });
});

describe("reselect", () => {
  it("keeps the previously selected alias after a refresh", () => {
    expect(reselect(hosts, "bastion")).toBe(3);
  });

  it("falls back to the first host, or none", () => {
    expect(reselect(hosts, "gone")).toBe(0);
    expect(reselect(hosts, undefined)).toBe(0);
    expect(reselect([], "x")).toBe(-1);
  });
});

describe("formatDetails", () => {
  it("joins user, hostname and port", () => {
    expect(formatDetails({ user: "deploy", hostname: "10.0.0.5", port: 2222 })).toBe("deploy@10.0.0.5:2222");
  });

  it("brackets IPv6 addresses before a port", () => {
    expect(formatDetails({ user: "root", hostname: "2001:db8::1", port: 22 })).toBe("root@[2001:db8::1]:22");
  });

  it("leaves out unresolved parts", () => {
    expect(formatDetails({ user: null, hostname: "example.com", port: null })).toBe("example.com");
  });
});

describe("configTarget", () => {
  const main = "C:\Users\me\.ssh\config";
  const inc = { file: "C:\Users\me\.ssh\conf.d\work", line: 7 };

  it("points at a host's only declaration", () => {
    expect(configTarget({ locations: [inc] }, main)).toEqual(inc);
  });

  it("falls back to the main config", () => {
    expect(configTarget(undefined, main)).toEqual({ file: main, line: null });
    expect(configTarget({ locations: [inc, { file: main, line: 2 }] }, main)).toEqual({ file: main, line: null });
    expect(configTarget({ locations: [] }, main)).toEqual({ file: main, line: null });
  });
});

describe("chooseConfig", () => {
  const def = String.raw`C:\Users\me\.ssh\config`;

  it("uses no -F for the default config", () => {
    expect(chooseConfig(def, def)).toBeUndefined();
    expect(chooseConfig("c:/users/ME/.ssh/config", def)).toBeUndefined();
  });

  it("keeps any other file", () => {
    const work = String.raw`D:\work\ssh config`;
    expect(chooseConfig(work, def)).toBe(work);
    expect(chooseConfig(work, "")).toBe(work);
  });
});
