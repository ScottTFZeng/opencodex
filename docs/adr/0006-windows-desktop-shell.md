# ADR 0006: Windows desktop delivery uses a Tauri shell with a staged Bun runtime

## Status

Accepted

## Context

The npm distribution requires Node to bootstrap Bun and opens the dashboard in a browser. Windows users need an installable application that starts the existing local dashboard directly, without a system Node, npm, or Bun installation. A standalone Bun executable would require the project to build its own window, tray, installer, updater, signing, and single-instance behavior.

## Decision

Use a Tauri v2 Windows shell with a physical Staged Runtime. Tauri owns the installer, native window, tray, single-instance behavior, code-signing integration, and updater. It starts or attaches to the Bun proxy, waits for an identity-checked health response, then loads the existing dashboard over loopback so its same-origin APIs and OAuth behavior remain unchanged.

The shell stops only an Owned Proxy. Scheduler and WinSW services remain optional Attached Proxies and are never enabled by default. Desktop Update replaces npm self-update for installed applications.

## Consequences

- The installed application carries Rust/Tauri build and updater maintenance.
- Bun source, dependencies, and dashboard assets must remain real staged files rather than archive-only frontend assets.
- Release automation must produce signed NSIS artifacts and signed updater metadata.
