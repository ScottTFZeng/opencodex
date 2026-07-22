import { readFileSync, writeFileSync } from "node:fs";
import { basename, resolve } from "node:path";

export type WindowsLatestJsonInput = {
  version: string;
  installerName: string;
  installerUrl: string;
  signature: string;
};

export function validateWindowsLatestJson(latest: unknown, input: WindowsLatestJsonInput): void {
  const expected = generateWindowsLatestJson(input);
  const platform = (latest as { platforms?: Record<string, { url?: unknown; signature?: unknown }> }).platforms?.["windows-x86_64"];
  if (!(latest as { version?: unknown }).version || (latest as { version?: unknown }).version !== expected.version
    || !platform || platform.url !== (expected.platforms as Record<string, { url: string }>)["windows-x86_64"].url
    || platform.signature !== (expected.platforms as Record<string, { signature: string }>)["windows-x86_64"].signature) {
    throw new Error("latest.json does not match the verified installer version, URL, and signature.");
  }
}

export function generateWindowsLatestJson(input: WindowsLatestJsonInput): Record<string, unknown> {
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(input.version)) throw new Error("Release version must be valid semver.");
  if (!/-setup\.exe$/i.test(input.installerName)) throw new Error("Installer must be an NSIS setup executable.");
  const url = new URL(input.installerUrl);
  const expectedPath = `/releases/download/v${input.version}/${input.installerName}`;
  if (url.protocol !== "https:" || !url.pathname.endsWith(expectedPath) || basename(url.pathname) !== input.installerName) {
    throw new Error("Installer URL must be the immutable HTTPS version-tag download URL.");
  }
  if (!input.signature) throw new Error("Updater signature is required.");
  return {
    version: input.version,
    notes: "",
    platforms: {
      "windows-x86_64": { url: url.toString(), signature: input.signature },
    },
  };
}

function env(name: string): string {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`${name} is required.`);
  return value;
}

if (import.meta.main) {
  const signaturePath = resolve(env("DESKTOP_INSTALLER_SIG_PATH"));
  const signature = readFileSync(signaturePath, "utf8");
  const input = {
    version: env("DESKTOP_RELEASE_VERSION"),
    installerName: env("DESKTOP_INSTALLER_NAME"),
    installerUrl: env("DESKTOP_INSTALLER_URL"),
    signature,
  };
  if (process.env.DESKTOP_VALIDATE_LATEST_JSON === "1") {
    validateWindowsLatestJson(JSON.parse(readFileSync(resolve(env("DESKTOP_LATEST_JSON_PATH")), "utf8")), input);
  } else {
    const outputPath = resolve(env("DESKTOP_LATEST_JSON_PATH"));
    writeFileSync(outputPath, `${JSON.stringify(generateWindowsLatestJson(input), null, 2)}\n`, "utf8");
  }
}
