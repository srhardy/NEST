# Resources needed — debloat + eggbot

## Access

- [x] **Spoke source** — received via Drive (325 MB zip). Full workspace on
      box at `spoke_src/spoke`; builds + 13 tests green on Rust 1.98.1.
      `nest` adapter shipped on branch `devin/nest-adapter`
      Still useful: push it to a private GitHub repo + add at
      app.devin.ai/settings/repositories so we can iterate with PRs.
- [ ] **Private app repository** — the real source (`@commandcode/desktop`,
      `command-code` engine, `@commandcode/harness`, `@commandcode/shared`,
      `@byokkit/*` providers). All code changes for both tracks live there;
      this public repo only holds installers/docs.
- [ ] **electron-builder config** (`electron-builder.yml` / `forge.config`)
      — to apply the packaging excludes, per-platform pruning, `asar`, and
      `electronLanguages` from `research/01`.
- [ ] **Engine API surface** — mostly mapped now via the shipped cli.mjs
      help text (research/05): agents are `.commandcode/agents/<name>.md`
      files, skills dirs, `.mcp.json` servers, `.commandcode/cron`.
      Remaining unknowns: exact `cron`/`trusted-hooks`/`settings` formats,
      and whether bots need a runtime IPC vs just file writes.
- [ ] **CI runners** for macOS (arm64 + x64), Windows x64, Linux x64 —
      required to verify installer fixes and packaging changes end-to-end.

## For the debloat track

- [ ] Decide supported UI languages → locale `.pak` allowlist
      (candidate: `en-US` only, matching the English-only docs).
- [ ] Confirm which `@jimp` operations are actually called (avatar/image
      resize?) → plugin subset or alternative.
- [ ] Confirm `@opentelemetry` usage is engine-side only → lazy-init or
      move behind the CLI process.
- [ ] Audit for other `.development.js` builds (found:
      `react-reconciler.development.js`).
- [ ] Verify nothing uses Vulkan/SwiftShader (WebGL fallbacks, GPU
      rasterization) before dropping `libvk_swiftshader`/`libvulkan`.
- [ ] Add installer smoke tests to CI using the existing
      `COMMANDCODE_INSTALL_DRY_RUN` / `_VERIFY_ONLY` hooks — would have
      caught the `gui`/`desktop` bug.

## For eggbot

- [ ] **Bot definition schema** — storage resolved: `~/.commandcode/agents/`
      (personal) vs `.commandcode/agents/` (project), matching skills' two
      tiers. Still open: extend `agent.toml` with a `[[bots]]`/`[[agents]]`
      section so the roster is manifest-declared, then have the adapter
      write the `.md` files.
- [ ] **Renderer surface** for the roster (new sidebar section) and the
      draft-card preview component (the app already has chat + settings
      primitives to reuse).
- [ ] **Routines**: does the engine have a scheduler/trigger API? If not,
      that's the largest new infra piece (cron runner + webhook endpoint +
      sender-key lifecycle via secret-request).
- [ ] **Memory**: decide scope — a per-bot notes file vs embedded store
      (drizzle is already in the bundle).
- [x] **Template format** — decision made: use Spoke `package.toml` instead
      of a bespoke `.ccbot`. Bot profile = a package carrying instructions +
      skills + routine manifest; secrets referenced by name only.
      (Remaining detail: extend `package.toml` for `agents/` entries.)
- [ ] **Design assets**: eggbot avatar (🥚 egg theme, mad-scientist), roster
      icons.
- [ ] Optional later: hosted template registry (categories, moderation,
      takedown flow) — explicitly out of the first build.

## Open questions for the team

1. Should bots be per-project or per-user global? (Grok: global roster,
   bots share the computer; Command Code is project-centric — likely global
   roster with an optional project binding.)
2. Is there an existing "agent"/"persona" concept in the engine to reuse,
   or is a bot a saved configuration that spawns normal sessions?
3. Routine execution model: run inside the desktop app while open, or
   always-on requires a background daemon/launch-agent?
4. Cap roster at 50 like Grok, or unlimited for local-only bots?
5. Marketplace: skip entirely at first (file/link sharing only) — confirm.
