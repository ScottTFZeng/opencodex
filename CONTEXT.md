# OpenCodex

OpenCodex is a local provider proxy for Codex and Claude Code, with a browser dashboard served by the proxy.

## Desktop Delivery

**Desktop Shell**:
The installed Windows application that owns the native window, tray, installer, and update lifecycle.
_Avoid_: GUI, proxy

**Owned Proxy**:
An OpenCodex process launched and identity-checked by the Desktop Shell. The shell may stop only this process.
_Avoid_: service, background process

**Attached Proxy**:
A healthy OpenCodex process started outside the Desktop Shell, such as a Scheduler or WinSW service. The shell can display it but never claims or stops it.
_Avoid_: owned proxy, child process

**Staged Runtime**:
The physical Bun executable, application source, production dependencies, and dashboard assets bundled with the installed Desktop Shell.
_Avoid_: compiled application, package archive

**Desktop Update**:
An installer-managed replacement of the Desktop Shell and its Staged Runtime.
_Avoid_: npm update, CLI update
