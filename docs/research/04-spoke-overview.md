# Spoke — Steven's plugin-management layer (from shared manifests)

Source files shared so far: workspace `Cargo.toml`, `Cargo.lock`, `README.md`.
Crate sources are on Steven's Mac (`~/projects/spoke`) — pending transfer.

## What it is

> "One hub for every coding agent. One manifest (`agent.toml`), one lockfile,
> one event log — projected into every harness you use."

Status **M0 ("Sync spoke")**: Claude Code and Codex adapters; commands
`init`, `sync`, `diff`, `doctor`. Rust workspace, ~61 locked crates,
light deps (clap, serde, toml_edit, sha2 — deliberately no async/network
stack yet; registry fetching is stubbed or minimal).

## Architecture

```
crates/spoke-core       manifest, projection IR, lockfile, paths
crates/spoke-adapters   per-harness adapters + safe apply/backup engine
crates/spoke-registry   package resolution, fetching, scanning, dedupe
crates/spoke-cli        the `spoke` binary
docs/                   ABOUT · MANIFEST · ADAPTER-CONTRACT ·
                        HARNESS-MATRIX · SECURITY
```

## File formats (per docs index)

- `agent.toml` — the user manifest: what agents/skills/MCPs/configs you want
- `package.toml` — a distributable plugin package (skill/MCP/config bundle)
- `index.toml` — registry index
- `agent.lock` — resolved/pinned state (lockfile)
- `~/.spoke/` — home layout incl. `backups/<unix-seconds>/`

## Design rules (verbatim from README — good ones)

- **Never clobber user config** — marked blocks
  (`<!-- spoke:begin ID --> … <!-- spoke:end ID -->`) in instruction files,
  or surgical JSON/TOML merges preserving unknown keys + formatting.
- **Backups before every write** — `~/.spoke/backups/`.
- **Warn, don't silently drop** — inexpressible projections surface via
  `spoke doctor` / `sync` warnings.
- **Idempotent** — second apply is a no-op.

## Why it's the "secret sauce" for this project

Spoke solves the *distribution* problem that Grok's templates solve on x.ai:
plugins, skills, and configs versioned, locked, and projected into any
harness — instead of trapped inside one tool. In the eggbot design it slots
in as the content layer:

- `spoke-registry` ↔ Grok marketplace/template store (packages = skills,
  MCPs, routines, bot profiles)
- `agent.toml` ↔ the bot roster manifest (declare bots + their contents)
- `spoke-adapters` ↔ harness projection (write a **Command Code adapter** and
  Spoke-managed content lands in the desktop app natively — and the same
  manifest still feeds Claude Code/Codex)
- marked blocks + backups ↔ Command Code's existing "never clobber"
  philosophy (same instinct as their installer fail-closed checks)

See `../plan/eggbot-upgrade-plan.md` § Spoke integration.


---

## Verified on box (2026-09-17)

- Rust 1.83 → 1.98.1 needed (clap 4.6.6 requires edition2024); `cargo build` +
  all 13 tests green.
- End-to-end run against a fake `HOME`: `init` → `diff` → `sync` → `doctor` →
  `add`/`list`/`why`; idempotency confirmed (second sync = 0 changed).
- **`nest` adapter shipped** — branch `devin/nest-adapter`,
  commits `282b7a0`+`480466d` (renamed command-code→nest after the NEST
  naming). Projects instructions→`AGENTS.md`, `mcp[]`→`.nest/mcp.json`,
  packages→`~/.nest/skills/<name>/`. trusted-hooks/permissions/mods/cron are
  reported as capability gaps, not written (formats unverified — see
  research/05).
- Exported for Steven's local copy: `research/command-code-adapter.patch` (original commit; rename applied in-tree)
  (apply with `git am`) and `spoke-src-adapter.zip` (full tree incl. .git).
