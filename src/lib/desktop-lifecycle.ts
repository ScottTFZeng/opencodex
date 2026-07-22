import { existsSync, readFileSync, rmSync } from "node:fs";

const CONTROL_FILE_ENV = "OCX_DESKTOP_CONTROL_FILE";
const CONTROL_TOKEN_ENV = "OCX_DESKTOP_CONTROL_TOKEN";

export type DesktopShutdownIo = {
  exists?: (path: string) => boolean;
  readFile?: (path: string) => string;
  removeFile?: (path: string) => void;
};

/**
 * The native shell owns a child through a private, random control file. This deliberately
 * avoids the shared management API: an attached proxy must never be stopped by the shell.
 */
export function consumeDesktopShutdownRequest(
  env: Record<string, string | undefined> = process.env,
  io: DesktopShutdownIo = {},
): boolean {
  const path = env[CONTROL_FILE_ENV]?.trim();
  const token = env[CONTROL_TOKEN_ENV]?.trim();
  if (!path || !token) return false;

  const exists = io.exists ?? existsSync;
  if (!exists(path)) return false;
  try {
    const readFile = io.readFile ?? (candidate => readFileSync(candidate, "utf8"));
    if (readFile(path).trim() !== token) return false;
    const removeFile = io.removeFile ?? (candidate => rmSync(candidate, { force: true }));
    removeFile(path);
    return true;
  } catch {
    return false;
  }
}

/** Starts the desktop-only shutdown watcher and returns its cleanup function. */
export function watchDesktopShutdownRequest(
  onShutdown: () => void,
  env: Record<string, string | undefined> = process.env,
  intervalMs = 200,
): () => void {
  if (!env[CONTROL_FILE_ENV]?.trim() || !env[CONTROL_TOKEN_ENV]?.trim()) return () => {};
  let consumed = false;
  const timer = setInterval(() => {
    if (consumed || !consumeDesktopShutdownRequest(env)) return;
    consumed = true;
    onShutdown();
  }, intervalMs);
  (timer as ReturnType<typeof setInterval> & { unref?: () => void }).unref?.();
  return () => clearInterval(timer);
}
