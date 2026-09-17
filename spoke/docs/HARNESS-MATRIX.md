# HARNESS-MATRIX.md — what each harness accepts, and what Spoke writes

Verified against each harness's official documentation. Harness formats move fast: every claim here is exercised by container tests when the adapter ships, and `spoke doctor` reports capability gaps live.

---

## 1. The compatibility matrix

| | Claude Code | Codex CLI | Nest | Gemini CLI | OpenCode |
|---|---|---|---|---|---|
| **Instructions** | `CLAUDE.md` (never `AGENTS.md`) | `AGENTS.md` | `AGENTS.md` (cross-tool convention) | `GEMINI.md` (configurable) | `AGENTS.md` (+ `CLAUDE.md` fallback) |
| **Skills** | `.claude/skills/<name>/SKILL.md` | `SKILL.md` + explicit `skills.config` entries | `.nest/skills/<name>/` + `~/.nest/skills/` | `.gemini/skills` or `.agents/skills` | `.opencode/skills` |
| **Commands** | Markdown (legacy `commands/*.md`) | `~/.codex/prompts/*.md` | skills invoked as `/<name>` | **TOML** commands | Markdown + frontmatter |
| **Hooks** | `settings.json`, ~30 events, 5 handler types | `config.toml` / `hooks.json`, command + MCP handlers | `.nest/trusted-hooks` (designed, unverified) | `settings.json`, `Before*/After*`, shell only | **none declarative** — JS plugins |
| **MCP** | `.mcp.json` → `{"mcpServers": …}` | `[mcp_servers.<id>]` TOML | `.nest/mcp.json` `mcpServers` (accepts `type`/`transport` = stdio\|http\|sse) — **same shape as Claude** | `settings.json` → `url` (SSE) vs `httpUrl` (HTTP) | `{"mcp": {"<name>": {"type":"local"\|"remote"}}}` |
| **Permissions** | `permissions.allow/ask/deny` rules | `sandbox_mode` + `approval_policy` + profiles | `.nest/settings.json` (designed) | `tools.*` + separate TOML policy engine | flat `permission` map (`allow\|ask\|deny`) |
| **Subagents / plugins** | `.claude/agents/*.md`, plugin bundles | TOML `[agents]`, git/local marketplaces | `.nest/agents/<name>.md` + `~/.nest/agents/` + `.nest/mods/*.ts` (trust-gated) | `.gemini/agents/*.md`, `gemini-extension.json` | `.opencode/agents/*.md`, JS plugins |
| **Trust gate** | workspace-trust dialog | project must be trusted | trust-gated mods (`.nest/mods`) | folder trust | none (precedence only) |

**The one portable unit is the skill.** All five read `SKILL.md`; everything else — instruction file names, hook models, permission syntax, MCP shapes — is genuinely incompatible. Spoke's response is to project per harness and warn when something cannot be represented, never to flatten everything to the lowest common denominator.

---

## 2. What Spoke writes today

| Projection | Claude Code | Codex CLI | Nest | Gemini CLI | OpenCode |
|---|---|---|---|---|---|
| Instruction bridge | **Shipped** — marked block in `CLAUDE.md` importing `AGENTS.md` | **Shipped** — marked block in `AGENTS.md` | **Shipped** — marked block in `AGENTS.md` (native memory file) | Planned | Planned |
| MCP servers | **Shipped** — `.mcp.json` `mcpServers.<name>` | **Shipped** — `.codex/config.toml` `[mcp_servers.<name>]` | **Shipped** — `.nest/mcp.json` `mcpServers.<name>` | Planned | Planned |
| Skills (packages) | **Shipped** — `~/.claude/skills/<name>/` | **Shipped** — `~/.codex/skills/<name>/` (⚠ see gap 1) | **Shipped** — `~/.nest/skills/<name>/` | Planned | Planned |
| Agents / bots | Not yet | Not yet | **Shipped** — `[[bots]]` → `.nest/agents/<name>.md`; package `[[agents]]` → `~/.nest/agents/<name>/` | Planned | Planned |
| Automations | Not yet | Not yet | **Shipped** — `[[automations]]` → `.nest/automations.toml` | Planned | Planned |
| Telemetry-off env | **Shipped** — merged `env` keys in `.claude/settings.json` | Not expressible (⚠ gap 2) | Not yet (settings format in design) | Planned | Planned |
| Accounting hooks | **Shipped** — append-only `SessionStart`/`SessionEnd`/`PreCompact` | Not yet (different hook model) | Not yet (`trusted-hooks` designed, format unverified) | Not yet | Not yet (no declarative hooks) |
| Permissions / policy | Planned (policy spoke) | Planned | Planned | Planned | Planned |

Scope is deliberate: two harnesses done properly beat six done badly. Adding a harness is one file plus the contract tests in `ADAPTER-CONTRACT.md` — every existing command starts working with it immediately. Nest was added in one adapter file because its surfaces (`AGENTS.md`, `.nest/mcp.json`, `~/.nest/skills/`, `.nest/agents/`, `.nest/automations.toml`) evolve the proven Command Code layout. Removing a `[[bots]]` entry does not yet delete the rendered `.nest/agents/<name>.md` — surfaced as a sync warning until stale tracking lands.

---

## 3. Known gaps (reported by `spoke doctor`, never hidden)

| # | Gap | Impact | Plan |
|---|---|---|---|
| 1 | Codex requires skills to be registered in `skills.config`; Spoke currently only writes the directory | A skill installed for Codex may not be picked up until registered | Register skills in `config.toml` when the Codex adapter handles config mutation (next pass) |
| 2 | Codex has no environment-projection surface for the telemetry-off variables | Beacons in a Codex environment are not silenced by Spoke | Provide a `spoke wrap` launcher (budget milestone) |
| 3 | OpenCode hooks are code, not configuration | Accounting hooks cannot be installed declaratively | Ship a versioned JS shim when OpenCode support lands |
| 4 | Gemini commands are TOML, everyone else's are Markdown | Command projection needs a per-harness renderer | Part of the Gemini adapter |
| 5 | Hooks are the least portable surface (event names, matchers and blocking semantics all differ) | Cross-harness hook parity is impossible; only intent maps | Map intent, warn on unmappable events |

---

## 4. Trust gates (why a synced file can appear to do nothing)

- **Claude Code** applies project-level hook/permission configuration only after the workspace-trust dialog is accepted.
- **Codex** ignores a project `.codex/config.toml` until the project is trusted (`projects.<path>.trust_level`).
- **Gemini** has folder trust plus a separate policy engine; distrust silently disables enforcement.
- **OpenCode** has no trust prompt — managed configuration is precedence-only, so a later config layer can override Spoke.

`spoke doctor` reports drift after the fact; if a projection looks applied but inert, the trust gate is the first thing to check.

---

## 5. Portability rules Spoke follows

| Rule | Rationale |
|---|---|
| Instructions are bridged, never duplicated | One canonical text (`AGENTS.md`), with each harness's file importing or containing the Spoke block |
| Skills are installed once per harness into its native skills root | All four read `SKILL.md`; only the directory differs |
| MCP servers are written in each harness's own shape and transport names | `url` vs `httpUrl` vs `type: "local"` are not interchangeable |
| Unmappable surfaces produce warnings | Silent drops are indistinguishable from bugs |
| Detection is directory-based, not assumed | An enabled-but-absent harness is skipped with a note rather than written into a void |
| Every write is idempotent | Re-running sync must be safe, so it can be run often |
