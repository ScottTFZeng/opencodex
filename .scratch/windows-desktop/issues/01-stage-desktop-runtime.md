# 01 — Stage the desktop runtime

**What to build:** Produce a checkable Windows desktop runtime payload containing the Bun executable, OpenCodex runtime, production dependencies, package metadata, and current dashboard output. The proxy must resolve the staged dashboard when launched in desktop mode, while existing npm and development behavior stays unchanged.

**Blocked by:** None — can start immediately.

**Status:** done

- [x] A build command creates a deterministic staged runtime and manifest with checksums.
- [x] A staged proxy serves the current dashboard through the existing same-origin routes.
- [x] Existing non-desktop GUI asset resolution and update behavior remain unchanged.
