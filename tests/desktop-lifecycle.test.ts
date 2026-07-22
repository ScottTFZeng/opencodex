import { describe, expect, test } from "bun:test";
import { consumeDesktopShutdownRequest } from "../src/lib/desktop-lifecycle";

const env = {
  OCX_DESKTOP_CONTROL_FILE: "C:\\temp\\desktop-stop",
  OCX_DESKTOP_CONTROL_TOKEN: "unpredictable-token",
};

describe("desktop shutdown control", () => {
  test("consumes only an exact token-matched request", () => {
    const removed: string[] = [];
    expect(consumeDesktopShutdownRequest(env, {
      exists: () => true,
      readFile: () => "unpredictable-token\n",
      removeFile: path => removed.push(path),
    })).toBe(true);
    expect(removed).toEqual(["C:\\temp\\desktop-stop"]);
  });

  test("rejects absent, malformed, and mismatched control state", () => {
    expect(consumeDesktopShutdownRequest({}, { exists: () => true })).toBe(false);
    expect(consumeDesktopShutdownRequest(env, { exists: () => false })).toBe(false);
    expect(consumeDesktopShutdownRequest(env, {
      exists: () => true,
      readFile: () => "another-process-token",
    })).toBe(false);
  });
});
