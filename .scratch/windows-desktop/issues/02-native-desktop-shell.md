# 02 — Build the native desktop shell

**What to build:** Deliver a single-instance Windows desktop application that starts or attaches to OpenCodex, waits for verified readiness, loads the existing dashboard, and safely manages tray, retry, logs, and exit behavior.

**Blocked by:** 01 — Stage the desktop runtime.

**Status:** implemented-pending-windows-validation

- [x] The application starts at most one Owned Proxy and attaches without claiming an Attached Proxy.
- [x] The dashboard opens only after verified readiness and startup failures remain actionable without it.
- [x] Close hides to tray; explicit exit stops only the Owned Proxy.

Windows validation remains required: Cargo, Tauri, NSIS, WebView2, ACL behavior, and real sidecar lifecycle could not run on this macOS host.
