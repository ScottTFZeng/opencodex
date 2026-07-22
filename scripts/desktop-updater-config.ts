import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = dirname(fileURLToPath(new URL("../package.json", import.meta.url)));
const DEFAULT_OUTPUT = join(ROOT, "src-tauri", "tauri.updater.release.conf.json");

export type DesktopUpdaterConfig = {
  bundle: { createUpdaterArtifacts: true; windows?: { signCommand: string } };
  plugins: { updater: { pubkey: string; endpoints: [string] } };
};

export function validateDesktopUpdaterEndpoint(endpoint: string): string {
  let url: URL;
  try { url = new URL(endpoint); } catch { throw new Error("TAURI_UPDATER_ENDPOINT must be an HTTPS URL."); }
  if (url.protocol !== "https:") throw new Error("TAURI_UPDATER_ENDPOINT must be an HTTPS URL.");
  return url.toString();
}

export function validateDesktopUpdaterPublicKey(pubkey: string): string {
  if (pubkey.length < 32 || !/^[A-Za-z0-9+/=]+$/.test(pubkey) || /^(?:example|test|placeholder|fake|changeme)/i.test(pubkey)) {
    throw new Error("TAURI_UPDATER_PUBKEY must be the release public key, not a placeholder.");
  }
  return pubkey;
}

export function resolveDesktopUpdaterConfig(env: Record<string, string | undefined> = process.env): DesktopUpdaterConfig {
  const pubkey = env.TAURI_UPDATER_PUBKEY?.trim();
  const endpoint = env.TAURI_UPDATER_ENDPOINT?.trim();
  if (!pubkey) throw new Error("TAURI_UPDATER_PUBKEY is required for a signed desktop release.");
  if (!endpoint) throw new Error("TAURI_UPDATER_ENDPOINT is required for a signed desktop release.");
  const signCommand = env.TAURI_WINDOWS_SIGN_COMMAND?.trim();
  if (env.TAURI_REQUIRE_WINDOWS_SIGN_COMMAND === "1" && !signCommand) {
    throw new Error("TAURI_WINDOWS_SIGN_COMMAND is required for a signed Windows release.");
  }
  return {
    bundle: { createUpdaterArtifacts: true, ...(signCommand ? { windows: { signCommand } } : {}) },
    plugins: { updater: { pubkey: validateDesktopUpdaterPublicKey(pubkey), endpoints: [validateDesktopUpdaterEndpoint(endpoint)] } },
  };
}

export function writeDesktopUpdaterConfig(outputPath = DEFAULT_OUTPUT, env: Record<string, string | undefined> = process.env): void {
  mkdirSync(dirname(outputPath), { recursive: true });
  writeFileSync(outputPath, `${JSON.stringify(resolveDesktopUpdaterConfig(env), null, 2)}\n`, "utf8");
}

if (import.meta.main) writeDesktopUpdaterConfig();
