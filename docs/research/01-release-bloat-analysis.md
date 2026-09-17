# Command Code release bloat analysis

Measured on `CommandCode-0.1.30-amd64.deb` (latest, 2026-09-16) and compared
with `v0.1.29`. Numbers are installed (unpacked) sizes unless noted.

## Size trend

| Release | .deb package | Installed | `resources/app/node_modules` |
|---------|-------------|-----------|------------------------------|
| 0.1.29  | 176 MB      | 762 MB    | 432 MB                       |
| 0.1.30  | 126 MB      | 557 MB    | 226 MB                       |

Between 0.1.29 → 0.1.30 the team already removed ~205 MB from `node_modules`:
`mermaid` (81 MB), `@img/*` sharp binaries (55 MB), `cytoscape` + `d3` stack
(~15 MB), `katex` (4.5 MB), `cmdk`, `@floating-ui`, `@iconify`, `@upsetjs`
and friends. Diagram rendering moved to lazy-loaded renderer chunks
(`out/renderer/assets/graph-*.js`, `sankeyDiagram-*.js`), which is the right
pattern — the advice below builds on it.

## Installed layout (v0.1.30, 557 MB)

| Component | Size | Notes |
|-----------|------|-------|
| `command-code` (Electron binary) | 211 MB | Stock Chromium/Electron runtime |
| `resources/app/node_modules` | 226 MB | The remaining actionable bloat |
| `resources/app/out` | 19 MB | Bundled main + renderer (already chunked) |
| `locales/` | 47 MB | **55 Electron locale .pak files** |
| `LICENSES.chromium.html` | 20 MB | Legally required — keep |
| `icudtl.dat` | 11 MB | ICU data; could subset but risky |
| graphics libs (`libGLESv2`, `libvk_swiftshader`, `libvulkan.so.1`, `libffmpeg`) | ~15.5 MB | Software Vulkan/SwiftShader likely unneeded |
| misc (`resources.pak`, snapshots, crashpad, sandbox) | ~9 MB | Required |

## Bloat items, ranked

### A. Wins that need only packaging config changes (~95–115 MB installed)

| Item | Waste | Action |
|------|-------|--------|
| Electron `locales/` | ~46 MB | Ship `en-US.pak` (+ any actually-supported languages). Standard electron-builder `electronLanguages` / afterPack filter. |
| Nested `node_modules` duplicates | ~75 MB across 47 nested trees | Version conflicts force duplicated deps: `zod` ships 3× (top 5.8 MB + harness 5.5 MB + command-code 5.5 MB), `@ai-sdk` duplicated in harness + command-code + providers, every `@jimp/plugin-*` carries a ~1.8 MB private copy, `@byokkit/*` providers each carry 5.5–6.4 MB. Align versions (dedupe/hoist) or pack with `npm dedupe`/pnpm hoisting before packaging. |
| `@crosscopy/clipboard-*` | ~15 MB per platform | All 7 native `.node` binaries ship in every package (darwin-universal/x64/arm64, win32-x64/arm64, linux-x64/arm64). Per-platform build should keep only its own. |
| `.map` files | ~13 MB | 2,792 sourcemaps inside shipped `node_modules` (jimp browser map 2.9 MB, `ai` maps 1.8 MB, otel maps). Exclude `**/*.map` in electron-builder `files`. |
| `.ts` sources (non-d.ts) | ~3.9 MB | 3,909 TypeScript source files shipped in production deps — packages bundling `src/` next to `dist/`. Exclude via packaging or upstream. |
| `@parcel/watcher-*` other platforms | ~2.2 MB | Keep only linux-x64-glibc on this artifact. |
| `@lydell/node-pty-*` other platforms | ~1.7 MB | Same: only linux-x64 needed here. |
| Dep docs (`README`/`CHANGELOG`/misc `.md`) | ~3.5 MB | Strip markdown — but **keep `LICENSE*` files** (license compliance). |

### B. Code/dependency decisions (~55–70 MB installed)

| Item | Size | Question to answer |
|------|------|--------------------|
| `@jimp` full suite | 32 MB | 19 plugins (print, threshold, rotate, resize, quantize, mask, flip, fisheye, displace, crop, cover, contain, color, circle…). If usage is avatar/thumbnail resize only, depend on the needed plugins or a leaner lib. (Sharp/`@img` was already removed in 0.1.30 — check which path replaced it before re-adding anything.) |
| `@opentelemetry` | 21 MB | Full SDK + semantic-conventions + OTLP exporters in the desktop client. If tracing is engine-side only, lazy-init or move instrumentation behind the CLI process; semantic-conventions alone is 7.6 MB. |
| `drizzle-orm` | 15 MB | Verify it is actually exercised in the desktop bundle; if only used by the engine for a narrow table, it may be replaceable — low priority. |
| `react-reconciler.development.js` | 0.7 MB | A **development build** is shipped (inside the ink/react-reconciler path). Ensure production bundling sets `NODE_ENV=production` so dev builds of all deps are excluded — worth an audit for other `.development.js` files. |
| `marked-terminal` (1.8 MB) + `marked` | ~2.5 MB | Terminal markdown renderer inside the desktop bundle — confirm it is needed in the GUI at all vs. only in the CLI engine. |

### C. Platform payload (structural)

| Item | Size | Notes |
|------|------|-------|
| `command-code` Electron binary | 211 MB | Chromium+Node. Only fix is a different shell (Tauri/native) — out of scope for a debloat pass. Ensure no debug/unstripped variant is being shipped (it's already stock Electron). |
| `libvk_swiftshader.so` + `libvulkan.so.1` + `libGLESv2` | ~13 MB | SwiftShader software Vulkan rarely needed for a 2D productivity UI; can be dropped from the package if WebGL fallback isn't required. Verify no feature relies on GPU rasterization first. |
| `icudtl.dat` | 11 MB | Can be subset with `icu4c` data filtering but breaks Intl formatting for removed locales — only do alongside the locales trim and keep parity. |

### D. Packaging hygiene

- **No `app.asar`** — the app ships as ~thousands of loose files under
  `resources/app/`. electron-builder `asar: true` (with `asarUnpack` for the
  native `.node` binaries + the engine entrypoints the installer verifies)
  shrinks file count massively, speeds install/update I/O, and stops
  loose `.map`/`.ts` files leaking. Note: the installer currently verifies
  `node_modules/@commandcode/harness/dist/index.js` and
  `command-code/dist/cli.mjs` exist on disk — those paths would need
  `asarUnpack` or a verification change.
- **Renderer already lazy-loads** (405 chunk files, 17 MB in `out/renderer`)
  — mermaid/shiki language packs are dynamic imports. Trimming the grammar
  list is possible but low-value; skip.

## Estimated result

Applying section A alone: ~95–115 MB installed reduction, roughly **30–45 MB
off each platform download**. Adding B: ~150–175 MB installed (~30%).
The 211 MB Electron binary and 20 MB `LICENSES.chromium.html` are floors.

## Suggested packaging checklist (electron-builder)

```yaml
# electron-builder.yml — illustrative
electronLanguages: ["en-US"]
files:
  - "!**/*.map"
  - "!**/*.ts"
  - "!**/*.tsx"
  - "!**/{README,CHANGELOG,HISTORY}*"
  - "!**/*.development.js"
asar: true
asarUnpack:
  - "**/*.node"
  - "node_modules/@commandcode/harness/dist/**"
  - "node_modules/command-code/dist/**"
# plus per-platform pruning of @crosscopy, @parcel/watcher-*, @lydell/node-pty-*
# (or rely on optionalDependencies + omitting foreign platform packages)
```
