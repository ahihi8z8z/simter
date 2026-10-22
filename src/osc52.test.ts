import { describe, expect, it } from "vitest";
import { osc52Text } from "./osc52";

const base64 = (text: string) => btoa(String.fromCharCode(...new TextEncoder().encode(text)));

describe("osc52Text", () => {
  it("decodes clipboard writes from tmux", () => {
    expect(osc52Text(`c;${base64("hello world")}`)).toBe("hello world");
    // tmux leaves the selection target empty by default.
    expect(osc52Text(`;${base64("line 1\nline 2")}`)).toBe("line 1\nline 2");
    expect(osc52Text(`c;${base64("máy chủ ✓")}`)).toBe("máy chủ ✓");
  });

  it("never answers clipboard queries", () => {
    expect(osc52Text("c;?")).toBeNull();
  });

  it("ignores malformed writes", () => {
    expect(osc52Text("c;")).toBeNull();
    expect(osc52Text("no separator")).toBeNull();
    expect(osc52Text("c;not base64!")).toBeNull();
    expect(osc52Text(`c;${btoa("\xff\xfe")}`)).toBeNull();
  });
});
