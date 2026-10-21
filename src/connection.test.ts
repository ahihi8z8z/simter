import { describe, expect, it } from "vitest";
import { describeExit } from "./connection";

describe("describeExit", () => {
  it("treats exit 0 as a normal end, e.g. logout or Byobu detach", () => {
    expect(describeExit({ code: 0, error: null })).toEqual({ state: "disconnected", message: "Session ended" });
  });

  it("treats 255 as an ssh failure", () => {
    const outcome = describeExit({ code: 255, error: null });
    expect(outcome.state).toBe("failed");
    expect(outcome.message).toMatch(/connect or authenticate/);
  });

  it("passes other codes through as the remote shell's status", () => {
    expect(describeExit({ code: 130, error: null })).toEqual({
      state: "disconnected",
      message: "Remote shell exited with code 130",
    });
  });

  it("reports an unknown exit status as a failure", () => {
    expect(describeExit({ code: null, error: "cannot get exit status: x" })).toEqual({
      state: "failed",
      message: "cannot get exit status: x",
    });
  });
});
