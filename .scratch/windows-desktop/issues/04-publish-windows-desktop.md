# 04 — Publish the Windows desktop installer

**What to build:** Make CI build and inspect the Windows NSIS installer, then provide a release path for Authenticode signing and signed updater metadata.

**Blocked by:** 01 — Stage the desktop runtime; 02 — Build the native desktop shell; 03 — Route desktop updates through the installer updater.

**Status:** implemented-pending-windows-validation

- [x] Windows CI and release workflows build, stage, verify, and publish the desktop artifact path.
- [x] Release automation separates ephemeral CI updater keys from protected Authenticode and updater signing hooks.
- [x] Artifact, signature, publisher, version, and updater metadata validation is implemented before publication.

Windows CI remains required to execute NSIS installation, startup/explicit-exit cleanup, real Minisign verification, Authenticode publisher checks, and protected GitHub release publication.
