import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = dirname(fileURLToPath(new URL("../package.json", import.meta.url)));
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

export type DesktopVersionMetadata = {
  packageVersion: string;
  cargoVersion: string;
  tauriVersionSource: string;
};

function packageVersion(): string {
  const version = (JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8")) as { version?: unknown }).version;
  if (typeof version !== "string" || !SEMVER.test(version)) throw new Error("package.json has an invalid version.");
  return version;
}

function cargoVersion(): string {
  const cargo = readFileSync(join(ROOT, "src-tauri", "Cargo.toml"), "utf8");
  const version = cargo.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
  if (!version || !SEMVER.test(version)) throw new Error("src-tauri/Cargo.toml has an invalid package version.");
  return version;
}

export function readDesktopVersionMetadata(): DesktopVersionMetadata {
  return {
    packageVersion: packageVersion(),
    cargoVersion: cargoVersion(),
    tauriVersionSource: (JSON.parse(readFileSync(join(ROOT, "src-tauri", "tauri.conf.json"), "utf8")) as { version?: unknown }).version as string,
  };
}

export function assertDesktopVersionMetadata(expectedVersion?: string): DesktopVersionMetadata {
  const metadata = readDesktopVersionMetadata();
  if (metadata.tauriVersionSource !== "../package.json") throw new Error("Tauri must use ../package.json as its version source.");
  if (metadata.cargoVersion !== metadata.packageVersion) throw new Error("Cargo and package.json versions must match.");
  if (expectedVersion && metadata.packageVersion !== expectedVersion) throw new Error("Requested release version does not match desktop metadata.");
  return metadata;
}

if (import.meta.main) {
  const expected = process.env.DESKTOP_EXPECTED_VERSION?.trim();
  process.stdout.write(`${JSON.stringify(assertDesktopVersionMetadata(expected || undefined))}\n`);
}
