import { afterAll, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { isWindowsX64Pe, stageDesktopRuntime } from "../scripts/prepare-desktop";
import { findGuiDist, resolveGuiFilePath } from "../src/server/gui-static";

const tmp = mkdtempSync(join(tmpdir(), "ocx-desktop-stage-"));
const NORMALIZED_TIMESTAMP_MS = Date.parse("2000-01-01T00:00:00.000Z");

function write(path: string, contents: string): void {
  mkdirSync(join(path, ".."), { recursive: true });
  writeFileSync(path, contents, "utf8");
}

function writePackage(root: string, version = "1.3.14"): void {
  write(join(root, "package.json"), JSON.stringify({
    name: "opencodex-fixture",
    version: "2.0.0",
    dependencies: { bun: version, "runtime-dependency": "1.0.0" },
  }));
}

function writeWindowsPe(path: string): void {
  const executable = Buffer.alloc(128);
  executable[0] = 0x4d;
  executable[1] = 0x5a;
  executable.writeUInt32LE(64, 0x3c);
  executable.write("PE\0\0", 64, "ascii");
  executable.writeUInt16LE(0x8664, 68);
  mkdirSync(join(path, ".."), { recursive: true });
  writeFileSync(path, executable);
}

function writeFixtureSource(root: string): void {
  writePackage(root);
  write(join(root, "bun.lock"), [
    '{',
    '  "packages": {',
    '    "bun": ["bun@1.3.14"],',
    '    "@oven/bun-windows-x64": ["@oven/bun-windows-x64@1.3.14"]',
    '  }',
    '}',
  ].join("\n"));
  write(join(root, "src", "cli", "index.ts"), "export {};\n");
  write(join(root, "bin", "ocx.mjs"), "export {};\n");
  write(join(root, "gui", "dist", "index.html"), "<main>dashboard</main>\n");
  write(join(root, "gui", "dist", "assets", "app.js"), "console.log('dashboard');\n");
}

function fixtureCommandRunner(version = "1.3.14", installedBunVersion = version) {
  const commands: Array<{ executable: string; args: string[]; cwd: string }> = [];
  return {
    commands,
    runCommand(command: { executable: string; args: string[]; cwd: string }) {
      commands.push(command);
      if (command.args[0] === "install") {
        const modules = join(command.cwd, "node_modules");
        write(join(modules, "bun", "package.json"), JSON.stringify({ name: "bun", version: installedBunVersion }));
        writeWindowsPe(join(modules, "bun", "bin", "bun.exe"));
        write(join(modules, "runtime-dependency", "package.json"), JSON.stringify({ name: "runtime-dependency", version: "1.0.0" }));
        write(join(modules, "runtime-dependency", "index.js"), "module.exports = 1;\n");
        return { exitCode: 0, stdout: "", stderr: "" };
      }
      return { exitCode: 0, stdout: `${version}\n`, stderr: "" };
    },
  };
}

afterAll(() => rmSync(tmp, { recursive: true, force: true }));

describe("desktop runtime staging", () => {
  test("uses an isolated frozen production install and produces a timestamp-normalized manifest", () => {
    const source = join(tmp, "source");
    const output = join(tmp, "output");
    writeFixtureSource(source);
    write(join(source, "node_modules", "ambient-only", "index.js"), "do not stage\n");
    const runner = fixtureCommandRunner();

    const first = stageDesktopRuntime({ sourceRoot: source, outputRoot: output, platform: "win32", runCommand: runner.runCommand });
    const firstManifest = readFileSync(join(output, "manifest.json"));
    const second = stageDesktopRuntime({ sourceRoot: source, outputRoot: output, platform: "win32", runCommand: fixtureCommandRunner().runCommand });

    expect(second).toEqual(first);
    expect(readFileSync(join(output, "manifest.json"))).toEqual(firstManifest);
    expect(runner.commands).toContainEqual({
      executable: process.execPath,
      args: ["install", "--production", "--frozen-lockfile"],
      cwd: `${output}.install`,
    });
    expect(runner.commands).toContainEqual({
      executable: join(`${output}.install`, "node_modules", "bun", "bin", "bun.exe"),
      args: ["--version"],
      cwd: `${output}.install`,
    });
    expect(existsSync(join(output, "binaries", "bun-x86_64-pc-windows-msvc.exe"))).toBe(true);
    expect(existsSync(join(output, "runtime", "bun.exe"))).toBe(false);
    expect(existsSync(join(output, "runtime", "node_modules", "runtime-dependency", "index.js"))).toBe(true);
    expect(existsSync(join(output, "runtime", "node_modules", "ambient-only"))).toBe(false);
    expect(existsSync(`${output}.install`)).toBe(false);
    expect(statSync(join(output, "runtime", "gui", "dist", "index.html")).mtimeMs).toBe(NORMALIZED_TIMESTAMP_MS);
    expect(second.files.map((file) => file.path)).toEqual([...second.files.map((file) => file.path)].sort());
    expect(second.files.some((file) => file.path === "manifest.json")).toBe(false);
  });

  test("rejects output and install roots that overlap the source before cleanup", () => {
    const source = join(tmp, "overlap-source");
    const sibling = join(tmp, "overlap-sibling");
    writeFixtureSource(source);
    const cases = [
      { outputRoot: source, installRoot: sibling },
      { outputRoot: sibling, installRoot: source },
      { outputRoot: join(source, "dist"), installRoot: sibling },
      { outputRoot: join(source, "dist", "desktop"), installRoot: sibling },
      { outputRoot: join(source, "other-output"), installRoot: sibling },
      { outputRoot: sibling, installRoot: join(source, "install") },
      { outputRoot: sibling, installRoot: join(source, "dist") },
      { outputRoot: sibling, installRoot: join(source, "dist", "desktop") },
      { outputRoot: tmp, installRoot: sibling },
      { outputRoot: sibling, installRoot: tmp },
    ];

    for (const paths of cases) {
      const runner = fixtureCommandRunner();
      expect(() => stageDesktopRuntime({
        sourceRoot: source,
        outputRoot: paths.outputRoot,
        installRoot: paths.installRoot,
        platform: "win32",
        runCommand: runner.runCommand,
      })).toThrow("must not overlap the source root");
      expect(existsSync(join(source, "package.json"))).toBe(true);
      expect(runner.commands).toHaveLength(0);
    }
  });

  test("allows only the repository-local desktop staging roots under the source", () => {
    const source = join(tmp, "allowed-staging-source");
    const output = join(source, "dist", "desktop", "windows-x64");
    writeFixtureSource(source);

    const manifest = stageDesktopRuntime({
      sourceRoot: source,
      outputRoot: output,
      installRoot: `${output}.install`,
      platform: "win32",
      runCommand: fixtureCommandRunner().runCommand,
    });

    expect(manifest.version).toBe("2.0.0");
    expect(existsSync(join(output, "manifest.json"))).toBe(true);
  });

  test("rejects an allowed lexical staging root with a symlinked dist ancestor before cleanup", () => {
    const source = join(tmp, "symlinked-staging-source");
    const outside = join(tmp, "symlinked-staging-outside");
    const output = join(source, "dist", "desktop", "windows-x64");
    const sentinel = join(outside, "sentinel.txt");
    const runner = fixtureCommandRunner();
    writeFixtureSource(source);
    write(sentinel, "must not be deleted\n");
    symlinkSync(outside, join(source, "dist"), "dir");

    expect(() => stageDesktopRuntime({
      sourceRoot: source,
      outputRoot: output,
      installRoot: `${output}.install`,
      platform: "win32",
      runCommand: runner.runCommand,
    })).toThrow("Symlinks or junctions are not allowed in desktop staging paths");
    expect(readFileSync(sentinel, "utf8")).toBe("must not be deleted\n");
    expect(runner.commands).toHaveLength(0);
  });

  test("rejects descendants of the allowed staging roots that could be deleted during staging", () => {
    const source = join(tmp, "staging-descendant-source");
    const output = join(source, "dist", "desktop", "windows-x64");
    writeFixtureSource(source);

    for (const paths of [
      { outputRoot: output, installRoot: join(output, "install") },
      { outputRoot: join(output, "nested"), installRoot: `${output}.install` },
      { outputRoot: output, installRoot: join(`${output}.install`, "nested") },
    ]) {
      expect(() => stageDesktopRuntime({
        sourceRoot: source,
        outputRoot: paths.outputRoot,
        installRoot: paths.installRoot,
        platform: "win32",
        runCommand: fixtureCommandRunner().runCommand,
      })).toThrow("must not overlap");
    }
  });

  test("rejects source symlinks that escape an allowlisted runtime root", () => {
    const source = join(tmp, "symlink-source");
    const output = join(tmp, "symlink-output");
    const outside = join(tmp, "outside.ts");
    writeFixtureSource(source);
    write(outside, "external content\n");
    symlinkSync(outside, join(source, "src", "escape.ts"));

    expect(() => stageDesktopRuntime({
      sourceRoot: source,
      outputRoot: output,
      platform: "win32",
      runCommand: fixtureCommandRunner().runCommand,
    })).toThrow("Symlinks are not allowed");
    expect(existsSync(join(output, "src", "escape.ts"))).toBe(false);
  });

  test("fails closed when the installed Bun runtime version differs from the pinned package version", () => {
    const source = join(tmp, "version-source");
    writeFixtureSource(source);

    expect(() => stageDesktopRuntime({
      sourceRoot: source,
      outputRoot: join(tmp, "version-output"),
      platform: "win32",
      runCommand: fixtureCommandRunner("1.3.15").runCommand,
    })).toThrow("does not match pinned version 1.3.14");
  });

  test("recognizes only AMD64 PE executables as Windows Bun candidates", () => {
    const executable = join(tmp, "fixture-bun.exe");
    writeWindowsPe(executable);
    expect(isWindowsX64Pe(executable)).toBe(true);
    write(executable, "not a PE executable");
    expect(isWindowsX64Pe(executable)).toBe(false);
  });

  test("requires a Windows host unless a Windows command/filesystem fixture is injected", () => {
    expect(() => stageDesktopRuntime({ sourceRoot: tmp, outputRoot: join(tmp, "not-windows"), platform: "darwin" }))
      .toThrow("must run on Windows");
  });
});

describe("staged GUI resolution", () => {
  test("resolves dashboard assets from the staged normal filesystem tree", () => {
    const source = join(tmp, "gui-source");
    const output = join(tmp, "gui-output");
    writeFixtureSource(source);
    stageDesktopRuntime({ sourceRoot: source, outputRoot: output, platform: "win32", runCommand: fixtureCommandRunner().runCommand });

    const stagedGui = join(output, "runtime", "gui", "dist");
    expect(findGuiDist({ OCX_GUI_DIST: stagedGui }, [])).toBe(stagedGui);
    expect(resolveGuiFilePath(stagedGui, "/assets/app.js")).toBe(join(stagedGui, "assets", "app.js"));
  });
});
