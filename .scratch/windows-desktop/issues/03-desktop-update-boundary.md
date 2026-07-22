# 03 — Route desktop updates through the installer updater

**What to build:** Give installed desktop users an updater-managed path that preserves proxy ownership during updates while retaining the current npm and CLI update behavior for existing users.

**Blocked by:** 02 — Build the native desktop shell.

**Status:** implemented-pending-windows-validation

- [x] Desktop update actions never invoke npm self-update.
- [x] An update does not stop or claim an Attached Proxy.
- [x] Existing npm and CLI update behavior remains isolated from desktop update handling.

Windows validation remains required for Tauri updater signatures, NSIS handoff, native dialog behavior, and owned-proxy recovery during a real installer update.
