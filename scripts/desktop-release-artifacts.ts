import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";

export type DesktopArtifactSet = {
  installer: string;
  signature: string;
  application: string;
  manifest: string;
  installerName: string;
  version: string;
  sha256: string;
};

type RuntimeManifest = {
  schemaVersion: number;
  platform: string;
  version: string;
  files: Array<{ path: string; size: number; sha256: string }>;
};

function filesUnder(root: string): string[] {
  return readdirSync(root, { withFileTypes: true }).flatMap(entry => {
    const path = join(root, entry.name);
    return entry.isDirectory() ? filesUnder(path) : entry.isFile() ? [path] : [];
  });
}

function sha256(path: string): string {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

export function validateDesktopRuntimeManifest(path: string, expectedVersion?: string): RuntimeManifest {
  const manifest = JSON.parse(readFileSync(path, "utf8")) as RuntimeManifest;
  if (manifest.schemaVersion !== 1 || manifest.platform !== "windows-x64" || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(manifest.version)) {
    throw new Error("Desktop runtime manifest has an invalid header.");
  }
  if (expectedVersion && manifest.version !== expectedVersion) throw new Error("Desktop runtime manifest version does not match the expected release version.");
  const runtimeRoot = dirname(resolve(path));
  if (!Array.isArray(manifest.files) || manifest.files.length === 0) throw new Error("Desktop runtime manifest has no files.");
  const paths = new Set<string>();
  for (const file of manifest.files) {
    if (!file || !/^[0-9a-f]{64}$/.test(file.sha256) || !Number.isInteger(file.size) || file.size < 0 || !file.path || file.path.startsWith("/") || file.path.includes("..")) {
      throw new Error("Desktop runtime manifest contains an invalid file entry.");
    }
    if (paths.has(file.path)) throw new Error("Desktop runtime manifest contains duplicate paths.");
    paths.add(file.path);
    const stagedPath = join(runtimeRoot, file.path);
    if (!existsSync(stagedPath) || !statSync(stagedPath).isFile() || statSync(stagedPath).size !== file.size || sha256(stagedPath) !== file.sha256) {
      throw new Error(`Desktop runtime manifest checksum does not match staged file: ${file.path}`);
    }
  }
  return manifest;
}

export function discoverDesktopArtifacts(bundleDirectory: string, manifestPath: string, applicationPath: string, expectedVersion?: string): DesktopArtifactSet {
  const manifest = validateDesktopRuntimeManifest(manifestPath, expectedVersion);
  const installers = filesUnder(bundleDirectory).filter(path => /-setup\.exe$/i.test(path));
  if (installers.length !== 1) throw new Error(`Expected exactly one NSIS setup executable, found ${installers.length}.`);
  const installer = installers[0];
  if (!installer.includes(manifest.version)) throw new Error("NSIS setup executable does not include the expected release version.");
  const signature = `${installer}.sig`;
  if (!existsSync(signature) || !statSync(signature).isFile() || statSync(signature).size === 0) {
    throw new Error("The NSIS setup executable is missing a non-empty matching .sig file.");
  }
  if (!existsSync(applicationPath) || !statSync(applicationPath).isFile() || !/\.exe$/i.test(applicationPath) || /-setup\.exe$/i.test(applicationPath)) {
    throw new Error("The built OpenCodex application executable is missing or invalid.");
  }
  return {
    installer,
    signature,
    application: applicationPath,
    manifest: manifestPath,
    installerName: basename(installer),
    version: manifest.version,
    sha256: sha256(installer),
  };
}

function option(name: string): string {
  const index = process.argv.indexOf(name);
  const value = index === -1 ? undefined : process.argv[index + 1];
  if (!value) throw new Error(`Missing ${name}.`);
  return resolve(value);
}

if (import.meta.main) {
  const versionIndex = process.argv.indexOf("--version");
  const expectedVersion = versionIndex === -1 ? undefined : process.argv[versionIndex + 1];
  if (versionIndex !== -1 && !expectedVersion) throw new Error("Missing --version value.");
  const artifacts = discoverDesktopArtifacts(option("--bundle"), option("--manifest"), option("--application"), expectedVersion);
  const output = process.argv.includes("--out") ? option("--out") : undefined;
  const json = `${JSON.stringify(artifacts, null, 2)}\n`;
  if (output) writeFileSync(output, json, "utf8");
  else process.stdout.write(json);
}
