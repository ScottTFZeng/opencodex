import {
  cpSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  utimesSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_ROOT = dirname(fileURLToPath(new URL("../package.json", import.meta.url)));
const DEFAULT_OUTPUT_ROOT = join(SCRIPT_ROOT, "dist", "desktop", "windows-x64");
const RUNTIME_DIRNAME = "runtime";
const SIDECAR_DIRNAME = "binaries";
const WINDOWS_X64_SIDECAR = "bun-x86_64-pc-windows-msvc.exe";
const NORMALIZED_TIMESTAMP = new Date("2000-01-01T00:00:00.000Z");

type PackageMetadata = {
  version?: string;
  dependencies?: Record<string, string>;
};

export type DesktopCommand = {
  executable: string;
  args: string[];
  cwd: string;
};

export type DesktopCommandResult = {
  exitCode: number;
  stdout: string;
  stderr: string;
};

export type DesktopRuntimeManifest = {
  schemaVersion: 1;
  platform: "windows-x64";
  version: string;
  files: Array<{ path: string; size: number; sha256: string }>;
};

export type StageDesktopRuntimeOptions = {
  sourceRoot?: string;
  outputRoot?: string;
  installRoot?: string;
  platform?: NodeJS.Platform;
  runCommand?: (command: DesktopCommand) => DesktopCommandResult;
};

function comparePaths(a: string, b: string): number {
  return a === b ? 0 : a < b ? -1 : 1;
}

function pathsOverlap(a: string, b: string): boolean {
  return a === b || a.startsWith(`${b}${sep}`) || b.startsWith(`${a}${sep}`);
}

function readPackage(path: string): PackageMetadata {
  return JSON.parse(readFileSync(path, "utf8")) as PackageMetadata;
}

function requireExactBunVersion(metadata: PackageMetadata, packagePath: string): string {
  const version = metadata.dependencies?.bun;
  if (!version || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error(`package.json must pin an exact Bun version: ${packagePath}`);
  }
  return version;
}

function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function validateBunLock(lockPath: string, version: string): void {
  const lock = readFileSync(lockPath, "utf8");
  const escapedVersion = escapeRegex(version);
  const bunPackage = new RegExp(`"bun"\\s*:\\s*\\["bun@${escapedVersion}"`);
  const windowsPackage = new RegExp(`"@oven/bun-windows-x64"\\s*:\\s*\\["@oven/bun-windows-x64@${escapedVersion}"`);
  if (!bunPackage.test(lock) || !windowsPackage.test(lock)) {
    throw new Error(`bun.lock does not resolve Bun ${version} for Windows x64.`);
  }
}

function assertNoSymlinks(root: string): void {
  const stat = lstatSync(root);
  if (stat.isSymbolicLink()) throw new Error(`Symlinks are not allowed in desktop runtime sources: ${root}`);
  if (!stat.isDirectory()) return;

  for (const entry of readdirSync(root).sort(comparePaths)) {
    assertNoSymlinks(join(root, entry));
  }
}

function assertPathHasNoSymlinks(root: string, path: string): void {
  const relativePath = relative(root, path);
  if (relativePath === ".." || relativePath.startsWith(`..${sep}`)) {
    throw new Error(`Desktop runtime source escapes its allowlisted root: ${path}`);
  }
  let current = root;
  if (lstatSync(current).isSymbolicLink()) {
    throw new Error(`Symlinks are not allowed in desktop runtime sources: ${current}`);
  }
  for (const segment of relativePath.split(sep).filter(Boolean)) {
    current = join(current, segment);
    if (lstatSync(current).isSymbolicLink()) {
      throw new Error(`Symlinks are not allowed in desktop runtime sources: ${current}`);
    }
  }
}

function assertRegularFile(path: string, description: string): void {
  if (!existsSync(path) || !lstatSync(path).isFile()) {
    throw new Error(`${description} is missing or is not a regular file: ${path}`);
  }
}

function copyRequired(source: string, destination: string): void {
  if (!existsSync(source)) throw new Error(`Required desktop runtime asset is missing: ${source}`);
  assertNoSymlinks(source);
  mkdirSync(dirname(destination), { recursive: true });
  cpSync(source, destination, { recursive: true, dereference: false, force: true, preserveTimestamps: false });
}

function defaultRunCommand(command: DesktopCommand): DesktopCommandResult {
  const result = Bun.spawnSync([command.executable, ...command.args], {
    cwd: command.cwd,
    stdout: "pipe",
    stderr: "pipe",
  });
  return {
    exitCode: result.exitCode,
    stdout: Buffer.from(result.stdout).toString("utf8"),
    stderr: Buffer.from(result.stderr).toString("utf8"),
  };
}

function runOrThrow(runCommand: (command: DesktopCommand) => DesktopCommandResult, command: DesktopCommand, description: string): DesktopCommandResult {
  const result = runCommand(command);
  if (result.exitCode !== 0) {
    throw new Error(`${description} failed: ${result.stderr.trim() || result.stdout.trim() || `exit ${result.exitCode}`}`);
  }
  return result;
}

export function isWindowsX64Pe(path: string): boolean {
  try {
    const file = readFileSync(path);
    if (file.length < 64 || file[0] !== 0x4d || file[1] !== 0x5a) return false;
    const peOffset = file.readUInt32LE(0x3c);
    return peOffset + 6 <= file.length
      && file.subarray(peOffset, peOffset + 4).equals(Buffer.from("PE\0\0"))
      && file.readUInt16LE(peOffset + 4) === 0x8664;
  } catch {
    return false;
  }
}

function normalizeTimestamps(root: string): void {
  const stat = lstatSync(root);
  if (stat.isDirectory()) {
    for (const entry of readdirSync(root).sort(comparePaths)) normalizeTimestamps(join(root, entry));
  }
  utimesSync(root, NORMALIZED_TIMESTAMP, NORMALIZED_TIMESTAMP);
}

function sha256(path: string): string {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function collectManifestFiles(root: string, directory = root): DesktopRuntimeManifest["files"] {
  const files: DesktopRuntimeManifest["files"] = [];
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => comparePaths(a.name, b.name))) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...collectManifestFiles(root, path));
    } else if (entry.isFile()) {
      const relativePath = relative(root, path).split(sep).join("/");
      files.push({ path: relativePath, size: statSync(path).size, sha256: sha256(path) });
    }
  }
  return files;
}

function installProductionDependencies(
  sourceRoot: string,
  installRoot: string,
  runCommand: (command: DesktopCommand) => DesktopCommandResult,
): void {
  rmSync(installRoot, { recursive: true, force: true });
  mkdirSync(installRoot, { recursive: true });
  copyRequired(join(sourceRoot, "package.json"), join(installRoot, "package.json"));
  copyRequired(join(sourceRoot, "bun.lock"), join(installRoot, "bun.lock"));
  runOrThrow(runCommand, {
    executable: process.execPath,
    args: ["install", "--production", "--frozen-lockfile"],
    cwd: installRoot,
  }, "Isolated production dependency install");
}

export function stageDesktopRuntime(options: StageDesktopRuntimeOptions = {}): DesktopRuntimeManifest {
  const sourceRoot = resolve(options.sourceRoot ?? SCRIPT_ROOT);
  const outputRoot = resolve(options.outputRoot ?? DEFAULT_OUTPUT_ROOT);
  const installRoot = resolve(options.installRoot ?? `${outputRoot}.install`);
  const platform = options.platform ?? process.platform;
  const runCommand = options.runCommand ?? defaultRunCommand;

  if (platform !== "win32") {
    throw new Error("Windows desktop runtime staging must run on Windows so Bun resolves the Windows x64 binary.");
  }
  if (pathsOverlap(sourceRoot, outputRoot) || pathsOverlap(sourceRoot, installRoot)) {
    throw new Error("Desktop output and install roots must not overlap the source root.");
  }
  if (installRoot === outputRoot || installRoot.startsWith(`${outputRoot}${sep}`) || outputRoot.startsWith(`${installRoot}${sep}`)) {
    throw new Error("Desktop install and output roots must not overlap.");
  }

  const packagePath = join(sourceRoot, "package.json");
  const lockPath = join(sourceRoot, "bun.lock");
  const runtimeSources = [packagePath, lockPath, join(sourceRoot, "src"), join(sourceRoot, "bin"), join(sourceRoot, "gui", "dist")];
  for (const path of runtimeSources) assertPathHasNoSymlinks(sourceRoot, path);
  assertRegularFile(packagePath, "Package metadata");
  assertRegularFile(lockPath, "Bun lockfile");
  const packageMetadata = readPackage(packagePath);
  if (!packageMetadata.version) throw new Error(`Package metadata is missing a version: ${packagePath}`);
  const bunVersion = requireExactBunVersion(packageMetadata, packagePath);
  validateBunLock(lockPath, bunVersion);

  try {
    installProductionDependencies(sourceRoot, installRoot, runCommand);
    const installedBunPackage = join(installRoot, "node_modules", "bun");
    assertNoSymlinks(join(installRoot, "node_modules"));
    const installedBunMetadata = readPackage(join(installedBunPackage, "package.json"));
    if (installedBunMetadata.version !== bunVersion) {
      throw new Error(`Installed Bun ${installedBunMetadata.version ?? "unknown"} does not match pinned version ${bunVersion}.`);
    }

    const bunExecutable = join(installedBunPackage, "bin", "bun.exe");
    assertRegularFile(bunExecutable, "Installed Windows Bun executable");
    if (!isWindowsX64Pe(bunExecutable)) {
      throw new Error(`Installed Bun executable is not an AMD64 PE file: ${bunExecutable}`);
    }
    const bunRuntime = runOrThrow(runCommand, {
      executable: bunExecutable,
      args: ["--version"],
      cwd: installRoot,
    }, "Windows Bun version check");
    if (bunRuntime.stdout.trim() !== bunVersion) {
      throw new Error(`Installed Bun runtime ${bunRuntime.stdout.trim() || "unknown"} does not match pinned version ${bunVersion}.`);
    }

    const runtimeRoot = join(outputRoot, RUNTIME_DIRNAME);
    const sidecarPath = join(outputRoot, SIDECAR_DIRNAME, WINDOWS_X64_SIDECAR);
    rmSync(outputRoot, { recursive: true, force: true });
    mkdirSync(outputRoot, { recursive: true });
    // Tauri bundles this tree as one physical resource. Bun is an external sidecar so it
    // remains executable rather than becoming part of the staged runtime resource.
    copyRequired(bunExecutable, sidecarPath);
    copyRequired(join(sourceRoot, "src"), join(runtimeRoot, "src"));
    copyRequired(join(sourceRoot, "bin"), join(runtimeRoot, "bin"));
    copyRequired(join(sourceRoot, "gui", "dist"), join(runtimeRoot, "gui", "dist"));
    copyRequired(packagePath, join(runtimeRoot, "package.json"));
    copyRequired(lockPath, join(runtimeRoot, "bun.lock"));
    copyRequired(join(installRoot, "node_modules"), join(runtimeRoot, "node_modules"));
    normalizeTimestamps(outputRoot);

    const manifest: DesktopRuntimeManifest = {
      schemaVersion: 1,
      platform: "windows-x64",
      version: packageMetadata.version,
      files: collectManifestFiles(outputRoot),
    };
    const manifestPath = join(outputRoot, "manifest.json");
    writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
    utimesSync(manifestPath, NORMALIZED_TIMESTAMP, NORMALIZED_TIMESTAMP);
    return manifest;
  } finally {
    rmSync(installRoot, { recursive: true, force: true });
  }
}

if (import.meta.main) {
  const manifest = stageDesktopRuntime();
  console.log(`Staged OpenCodex ${manifest.version} desktop runtime with ${manifest.files.length} files.`);
}
