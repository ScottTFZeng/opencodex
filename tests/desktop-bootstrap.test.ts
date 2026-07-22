import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolveDesktopUpdaterConfig, validateDesktopUpdaterEndpoint, validateDesktopUpdaterPublicKey } from "../scripts/desktop-updater-config";
import { validateDesktopBootstrapSource } from "../scripts/build-desktop-bootstrap";

describe("desktop bootstrap boundary", () => {
  test("uses the bundled API client instead of the global bridge", () => {
    const source = readFileSync("src-tauri/bootstrap/bootstrap.ts", "utf8");
    expect(() => validateDesktopBootstrapSource(source)).not.toThrow();
    expect(source).not.toContain("window.__TAURI__");
  });

  test("limits the bridge to the local bootstrap window", () => {
    const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
    const capability = JSON.parse(readFileSync("src-tauri/capabilities/default.json", "utf8"));
    expect(config.app.security.withGlobalTauri).toBeUndefined();
    expect(capability.windows).toEqual(["main"]);
    expect(capability.remote).toBeUndefined();
    expect(capability.permissions).toEqual(["core:event:default"]);
  });

  test("rejects a bootstrap source that reaches for the global bridge", () => {
    expect(() => validateDesktopBootstrapSource('const api = window.__TAURI__;')).toThrow("global Tauri bridge");
  });
});

describe("desktop updater release config", () => {
  test("requires a public key and HTTPS endpoint at release time", () => {
    expect(() => resolveDesktopUpdaterConfig({ TAURI_UPDATER_ENDPOINT: "https://updates.example.test/latest.json" })).toThrow("TAURI_UPDATER_PUBKEY");
    expect(() => resolveDesktopUpdaterConfig({})).toThrow("TAURI_UPDATER_PUBKEY");
    expect(() => validateDesktopUpdaterEndpoint("http://updates.example.test/latest.json")).toThrow("HTTPS");
    expect(validateDesktopUpdaterEndpoint("https://updates.example.test/latest.json")).toBe("https://updates.example.test/latest.json");
    expect(() => validateDesktopUpdaterPublicKey("not-a-release-key")).toThrow("release public key");
  });

  test("desktop builds generate the updater config before staging or invoking Tauri", () => {
    const packageJson = JSON.parse(readFileSync("package.json", "utf8"));
    expect(packageJson.scripts["desktop:build"]).toContain("prepare:desktop-updater");
    expect(packageJson.scripts["desktop:build"]).toContain("--config src-tauri/tauri.updater.release.conf.json");
  });
});
