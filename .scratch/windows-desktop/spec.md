# Windows Desktop Delivery

**Status:** ready-for-agent

## Problem Statement

Windows users currently need Node and npm before installing OpenCodex. They then start a command-line bootstrap that opens a browser dashboard. This creates a high setup barrier for users who need a directly installable local application.

## Solution

Deliver a signed Windows x64 NSIS application that includes the OpenCodex runtime and opens the existing dashboard in its own native window. The application manages only the proxy it starts, uses the existing dashboard and same-origin API model, and replaces npm-based self-update with an installer-managed update channel.

## User Stories

1. As a Windows user, I want to install OpenCodex without installing Node, npm, or Bun, so that I can begin from one installer.
2. As a Windows user, I want the installed application to open OpenCodex directly, so that I do not need terminal commands or a separate browser window.
3. As a Windows user, I want the dashboard to appear only after the local proxy is ready, so that I do not see a broken management screen during startup.
4. As a Windows user, I want startup failures to expose retry, logs, and exit actions, so that I can recover without command-line diagnosis.
5. As a Windows user, I want closing the window to keep OpenCodex available in the notification area, so that routine window management does not interrupt work.
6. As a Windows user, I want an explicit exit to stop only the application-owned proxy, so that externally managed OpenCodex services keep running.
7. As a Windows user, I want a second launch to focus the existing application, so that it never creates a duplicate proxy.
8. As an existing service user, I want the application to attach to a healthy Scheduler or WinSW proxy, so that installation does not disrupt my existing configuration.
9. As a Windows user, I want desktop updates to be signed and installer-managed, so that upgrades do not depend on npm and are trustworthy.
10. As a release engineer, I want repeatable unsigned CI artifacts and separately signed release artifacts, so that packaging failures are caught before publication.
11. As a support engineer, I want runtime staging checksums and startup logs, so that installed-artifact problems are diagnosable.
12. As a Codex user, I want the existing proxy APIs and dashboard behavior to remain unchanged, so that moving to the desktop application does not alter provider configuration or authentication workflows.

## Implementation Decisions

- Use a Tauri v2 shell for Windows x64 packaging, native lifecycle, tray behavior, single-instance handling, and update integration.
- Retain Bun as the OpenCodex runtime and stage its executable, source, production dependencies, package metadata, and dashboard output as physical resources inside the installed application.
- Load the current dashboard from the Bun proxy's loopback address after an identity-checked health response; do not duplicate the dashboard inside the Tauri frontend protocol.
- Distinguish an Owned Proxy from an Attached Proxy. The shell can stop only an Owned Proxy and must leave Attached Proxies untouched.
- Start the proxy when the user opens the application. Closing hides the window to tray; explicit exit terminates the Owned Proxy after a graceful drain. Scheduler and WinSW remain opt-in services.
- Route desktop update actions to the Tauri updater. Preserve npm/CLI update behavior for the existing npm distribution.
- Produce NSIS install artifacts, Authenticode-sign release executables/installers, and publish separately signed updater metadata. CI builds remain unsigned.
- This removes the OpenCodex Node prerequisite only. Codex integration remains a separate dependency that the user configures as today.

## Testing Decisions

- Treat runtime staging, proxy ownership, and health readiness as public test seams; do not test private implementation order.
- Unit-test resource manifest generation, staged dashboard resolution, identity-aware attach/start/stop decisions, and desktop-mode update routing.
- Build an unsigned Windows installer in CI, inspect its staged resource manifest, launch it on Windows, wait for the real health endpoint, and verify explicit exit cleans up only the Owned Proxy.
- Keep existing TypeScript, GUI lint, localization, privacy, and proxy tests as regression coverage.

## Out of Scope

- Windows ARM64 delivery.
- Enabling or redesigning Scheduler and WinSW service management in the first desktop release.
- Rewriting the React dashboard or provider APIs.
- Bundling or configuring Codex itself.
- Automatic migration of existing npm installations into the desktop application.

## Further Notes

The first delivery should favor a staged runtime over a Bun compiled executable because current dynamic imports, service paths, and static asset serving depend on real files. A future compiled runtime can be evaluated after equivalent installer and service tests exist.
