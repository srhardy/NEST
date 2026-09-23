---
name: testing-nest
description: How to run and end-to-end test the NEST engine + web shell and the spoke CLI locally — ports, perch stubs, ~/.nest layout, deterministic hard-won/failure triggers, and spoke add requirements.
---

# Testing NEST (nest engine + shell, spoke CLI)

## Build & run
- `nest` binary: `cargo build` in `nest/` → `nest/target/debug/nest`. `spoke`: `cargo build` in `spoke/` → `spoke/target/debug/spoke`.
- Web shell: `nest shell` serves the SPA + JSON API on `http://127.0.0.1:4747` (default; `--port <n>` to change). It is blocking — run in background.
- Useful API: `GET /api/state` (roster+perches), `GET /api/runs` (receipts), `POST /api/hatch {"bot","task","perch"}`, `POST /api/brood {"task","perch"}`, `POST /api/feather {"run_id","name"?,"force"?}`, `GET /api/runs/<id>/events` (SSE; ends with `event: receipt` whose JSON has `hard_won`).
- UI chat input → `/api/hatch` when a roster bot is clicked, else `/api/brood`. The UI has NO perch picker — it always uses `perch::default_perch()` = first real harness binary found on PATH (`claude`, `codex`, `opencode`, in that order), else builtin `echo`.

## Perches & deterministic outcomes
- `echo` (builtin): prints prompt, exit 0 → every run `done`. `fail` (builtin): exit 1 → every run `failed`. Pass `--perch echo|fail` on CLI, or `"perch":"echo"` in POST bodies.
- Steps/budget: only `Peep::Step` events increment the step counter; echo/opencode-style perches emit only `Output` → `max_steps` can't trip on them.
- Brood dispatch shares ONE perch across all candidates, so a hard-won (fail-then-succeed) dispatch needs per-agent failure driven by prompt content: drop an executable stub named `opencode` (or `claude`/`codex`) on PATH (e.g. `~/.local/bin`) that exits 1 when an arg contains a marker placed ONLY in the failing bot's agent body, else exits 0. `default_perch()` picks it automatically in UI and CLI.

## ~/.nest layout (home; cwd `.nest/` project dir shadows it — run from a clean cwd or pass `--home`/`NEST_HOME`)
- `agents/<name>.md` — `---` YAML frontmatter (`name`, `job`, `boss: true`, `permissions`, `skills`, `max_steps`) + persona body. A `boss: true` bot suppresses the shell's first-run onboarding modal; `eggbot` is auto-seeded on first serve.
- `routing.toml` — `[[route]] patterns = ["kw"] bot = "a" fallback = ["b"]` (matches on task substring, lowercase).
- `roost/<receipt-id>.json` — every run writes one, including failures.
- `skills/<name>/SKILL.md` — feather output; frontmatter `name:`, `authored-by:`, `run:`.
- No credentials needed anywhere: with no `providers.toml`, the forage router falls back to the echo provider (distill output = the prompt reflected back).

## spoke CLI gotchas
- `spoke add <spec>` requires `agent.toml` in the project dir — `spoke init` in a scratch dir first.
- Installs only into harnesses whose `root(home)` dir exists: Nest→`~/.nest`, Claude Code→`~/.claude`, Codex→`~/.codex`. `mkdir ~/.claude` to make the Claude Code target visible and prove cross-harness installs land in `~/.claude/skills/<name>/`.
- `spoke add <bare-name>` resolves `~/.nest/skills/<name>` (bot-authored wrap); `spoke add <dir-with-SKILL.md>` also wraps; anything else falls through to the registry index at `~/.spoke/registry/index.toml` (missing index = clean error). Lockfile: `<project>/agent.lock`.

## Devin Secrets Needed
None — echo perch + echo forage provider cover everything; real LLM CLIs are optional.
