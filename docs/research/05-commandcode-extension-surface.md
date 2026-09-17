# 05 — Seed-engine extension surface → what NEST evolves

> **NEST note:** this documents the Command Code seed engine's verified
> formats. NEST moves the app-owned files under `.nest/` (`.nest/skills/`,
> `.nest/agents/`, `.nest/mcp.json`, `.nest/cron`, `.nest/mods/`,
> `.nest/trusted-hooks`, `.nest/settings.json`, `.nest/taste/`), keeping
> `AGENTS.md` at project root (cross-tool convention) and the same JSON
> entry shapes. The `nest` Spoke adapter already implements this.
>
> Original research follows, verbatim.

# 05 — Command Code extension surface (verified against shipped engine)

The app source is private, but `command-code/dist/cli.mjs` inside the installed
`.deb` documents the real extension formats in its `--help` output and embedded
constants. Verified on the shipped v0.1.30 engine. This is the map eggbot and
the Spoke adapter are built on.

## Surfaces the engine actually reads

| Surface | Location | Format | Notes |
|---|---|---|---|
| Project memory | `AGENTS.md` | markdown | `MEMORY_FILENAME = "AGENTS.md"` — read natively |
| Personal skills | `~/.commandcode/skills/<name>/` | `SKILL.md` dir | invoked as `/<name>` |
| Project skills | `<project>/.commandcode/skills/<name>/` | `SKILL.md` dir | invoked as `/<name>` |
| Compat skills | `<project>/.agents/skills/` | `SKILL.md` dir | legacy/compat path |
| Agents (sub-agents) | `.commandcode/agents/<name>.md`, `~/.commandcode/agents/<name>.md` | markdown | for `task` tool; model pinning; "create-an-agent-on-request" recipe built in |
| MCP servers | `.mcp.json` `mcpServers.<name>` | JSON | scope precedence: user `~/.commandcode/mcp.json` < project `.mcp.json` < local `~/.commandcode/projects/<slug>/mcp.json` |
| Cron | `.commandcode/cron` | unverified | exists |
| Hooks | `.commandcode/trusted-hooks` | unverified | exists |
| Mods | `.commandcode/mods/<name>.ts` | TypeScript | trust-gated |
| Taste | `.commandcode/taste/` | engine-managed | **do not write** — cli.mjs warns explicitly |

## MCP entry shape (verified in `inferTransport` / `toServerConfig`)

- Transport declared via `transport` or `type`, lowercased: `stdio | http | sse`.
- Inference: `url` && no `command` → `"http"`; otherwise `"stdio"`.
- `stdio` requires `command`; `http`/`sse` requires `url`.
- Consequence: the Claude-Code-shaped entry
  `{ "type": "stdio", "command": "...", "args": [...] }` and
  `{ "url": "..." }` are both valid verbatim → **Spoke's `.mcp.json` merge
  works unchanged.**

## What this means for eggbot

Nearly every Grok-bot substrate already exists natively:

| Grok bot concept | Command Code equivalent |
|---|---|
| bot profile + persona | `.commandcode/agents/<name>.md` agent file (or a bot schema projected to it) |
| bot skills | `.commandcode/skills/<name>/SKILL.md` |
| bot tools/integrations | `.mcp.json` `mcpServers` entries |
| routines (cron/webhook) | `.commandcode/cron` (format TBD) |
| bot instructions/memory | `AGENTS.md` + agent file frontmatter |
| plugins | `.commandcode/mods/*.ts` (trust-gated — good fit for opt-in extensions) |
| sharing (templates/marketplace) | **gap →** Spoke `package.toml` + registry |
| roster/sidebar | **gap →** eggbot UX layer |

## Spoke adapter mapping (shipped — `crates/spoke-adapters/src/command_code.rs`)

| Manifest field | Projection |
|---|---|
| `instructions.extra` | marked block in `AGENTS.md` |
| `mcp[]` | `mcpServers.<name>` merge into project `.mcp.json` |
| packages (skills) | files into `~/.commandcode/skills/<name>/` |
| hooks, permissions, mods, cron | not projected — warned as capability gaps |

Unverified formats (cron, trusted-hooks, settings/permissions) are intentionally
not written — per Spoke's warn-don't-guess rule.
