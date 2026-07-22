import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = dirname(fileURLToPath(new URL("../package.json", import.meta.url)));
const bootstrapDir = join(ROOT, "src-tauri", "bootstrap");
const sourcePath = join(bootstrapDir, "bootstrap.ts");
const outputPath = join(bootstrapDir, "bootstrap.js");

export function validateDesktopBootstrapSource(source: string): void {
  if (source.includes("window.__TAURI__")) throw new Error("Desktop bootstrap must not use the global Tauri bridge.");
  if (!source.includes('from "@tauri-apps/api/core"') || !source.includes('from "@tauri-apps/api/event"')) {
    throw new Error("Desktop bootstrap must import the local Tauri API client.");
  }
}

export async function buildDesktopBootstrap(): Promise<void> {
  validateDesktopBootstrapSource(readFileSync(sourcePath, "utf8"));
  const result = await Bun.build({
    entrypoints: [sourcePath],
    outdir: bootstrapDir,
    naming: "bootstrap.js",
    format: "esm",
    target: "browser",
    minify: true,
  });
  if (!result.success || !existsSync(outputPath)) {
    throw new Error(`Desktop bootstrap bundle failed: ${result.logs.map(log => log.message).join("\n")}`);
  }
}

if (import.meta.main) await buildDesktopBootstrap();
