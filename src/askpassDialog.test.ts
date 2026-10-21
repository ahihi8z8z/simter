import { describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({ askpassReply: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ message: vi.fn() }));

import { describeQuestion } from "./askpassDialog";

const base = { id: 1, alias: "mec2", prompt: "user@host's password: ", error: null };

describe("describeQuestion", () => {
  it("names the host and notes a rejected password", () => {
    expect(describeQuestion({ ...base, secret: true, retry: false })).toEqual({
      title: "mec2: password needed",
      note: null,
      accept: "OK",
    });
    expect(describeQuestion({ ...base, secret: true, retry: true }).note).toMatch(/not accepted/);
  });

  it("asks to accept a confirmation such as a new host key", () => {
    const d = describeQuestion({ ...base, prompt: "continue connecting (yes/no)?", secret: false, retry: false });
    expect(d.accept).toBe("Accept");
    expect(d.note).toBeNull();
  });
});
