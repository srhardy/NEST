---
name: testing-nest
description: How to run and end-to-end test the NEST engine + web shell and the spoke CLI locally — ports, perch stubs, ~/.nest layout, deterministic hard-won/failure triggers, and spoke add requirements.
---

# Testing NEST (nest engine + shell, spoke CLI)

## Build & run
- `nest` binary: `cargo build` in `nest/` → `nest/target/debug/nest`. `spoke`: `cargo build` in `spoke/` → `spoke/target/debug/spoke`.
- Web shell: `nest shell` serves the SPA + JSON API on `http://127.0.0.1:4747` (default; `--port <n>` to change). It is blocking — run in background.
- Useful API: `GET /api/state` (roster+perches), `GET /api/runs` (receipts), `POST /api/hatch {"bot","task","perch"}`, `POST /api/brood {"task","perch"}`, `POST /api/feather {"run_id","name"?,"force"?}`, `GET /api/runs/<id>/events` (SSE; ends with `event: receipt` whose JSON has `hard_won`).
- UI chat input → `/api/hatch` when a roster bot is clicked, else `/api/sessions/send` with `bot:"nest"` → **the chief** plans first (forage router): a provider reply of `{"answer":…}` answers directly (receipt `agent:"chief"`), `{"delegate":[{"bot":…}]}` hatches that bot via `Via::Chief`, anything else `Undecide`s into `routing.toml`. No providers.toml → echo forage echoes the prompt → always Undecided → deterministic routes. Script the chief's reply with a `kind = "command"` provider whose `program` printf's the JSON. `approved_tools: [...]` on send/hatch/brood promotes `ask`-policy tools for that run; unapproved asks emit a `Peep::Ask` and stay denied. The UI has NO perch picker — it always uses `perch::default_perch()` = first real harness binary found on PATH (`claude`, `codex`, `opencode`, in that order), else builtin `echo`.

## Perches & deterministic outcomes
- `echo` (builtin): prints prompt, exit 0 → every run `done`. `fail` (builtin): exit 1 → every run `failed`. Pass `--perch echo|fail` on CLI, or `"perch":"echo"` in POST bodies.
- Steps/budget: only `Peep::Step` events increment the step counter; echo/opencode-style perches emit only `Output` → `max_steps` can't trip on them. `max_seconds` (default 600) is the wall-clock rail that works everywhere — a killed run emits `Peep::TimedOut` and lands `BudgetExceeded`/`failure_kind: budget`. Kills take the child's whole process group (unix).
- Brood dispatch shares ONE perch across all candidates, so a hard-won (fail-then-succeed) dispatch needs per-agent failure driven by prompt content: drop an executable stub named `opencode` (or `claude`/`codex`) on PATH (e.g. `~/.local/bin`) that exits 1 when an arg contains a marker placed ONLY in the failing bot's agent body, else exits 0. `default_perch()` picks it automatically in UI and CLI.

## ~/.nest layout (home; cwd `.nest/` project dir shadows it — run from a clean cwd or pass `--home`/`NEST_HOME`)
- `agents/<name>.md` — `---` YAML frontmatter (`name`, `job`, `boss: true`, `permissions`, `skills`, `max_steps`) + persona body. A `boss: true` bot suppresses the shell's first-run onboarding modal; `eggbot` is auto-seeded on first serve.
- `routing.toml` — `[[route]] patterns = ["kw"] bot = "a" fallback = ["b"]` (matches on task substring, lowercase).
- `roost/<receipt-id>.json` — every run writes one, including failures; `failure_kind` says why (spawn/exit/budget/denied).
- `roost/<receipt-id>.peeps.jsonl` — the run's full event tape (one JSON per line, `ms` stamped); the receipt is just its summary.
- `skills/<name>/SKILL.md` — feather output; frontmatter `name:`, `authored-by:`, `run:`.
- No credentials needed anywhere: with no `providers.toml`, the forage router falls back to the echo provider (distill output = the prompt reflected back).

## Webhook daemon (`nest daemon --webhook <addr>`)
- One process runs BOTH the remotes watcher (~3s tick) and the HTTP listener; start it from a writable cwd (e.g. `/tmp/nest-cwd`) with `nohup` and check the log for `webhook on <addr>`.
- Remote fixture: `~/.nest/remotes/<name>/token` (raw token string) + `inbox/` dir. Every matched path needs `Authorization: Bearer <token>` — missing/wrong token or ghost remote → 403.
- POST `chat: <id>` or `channel: <id>` as the FIRST line of the body → reply lands at `outbox/<id>.txt` and `GET /inbox/<remote>/<id>/reply` serves it (404 pending → 200). No such line → reply keyed under the returned `{"id"}` (task file stem).
- IMPORTANT for curl fixtures: `-d 'chat: 42\n…'` sends a LITERAL `\n` — the whole body is one line and `reply_to` swallows everything after `chat: ` into the filename. Use bash ANSI quoting `-d $'chat: 42\n…'` for a real newline.
- Caps (verified at de23545): body >64KiB → 413; ≥64 pending `inbox/*.md` → 429. Seed the inbox with ~80 dummy `.md` files and POST immediately — the watcher drains every ~3s so a marginal count can race below the cap.
- Security probe: `chat:`/`channel:` values now pass `name_ok` like URL segments — `../`, absolute, backslash, subdir, and dotted values get no outbox write (fixed; locked in by `webhook_traversal_chat_id_writes_no_outbox`).

## spoke registry (`spoke registry sync/show`)
- `~/.spoke/registry` must NOT pre-exist as a non-git dir for the clone test — `rm -rf` it first (but check it's not a real checkout you want to keep).
- Local fixture registry: git repo containing `index.toml` (`[[package]] name = "x" manifest = "pkg/x/package.toml"`) + package dirs with `package.toml` (`[source] git = "<repo>" rev = "<sha>"` or `path = "."`; `[[skills]] name path`).
- sha-pin check: use a 2-commit package repo, pin `rev` to the FIRST commit sha, and grep the installed SKILL.md for a marker that only exists in v1 — proves full-clone+checkout (a broken pin installs v2/HEAD).
- Bare `sync` (existing checkout) does `git pull --ff-only` and ignores any url arg; commit a new package into the fixture registry to prove the pull path end-to-end.

## Devin Secrets Needed
None — echo perch + echo forage provider cover everything; real LLM CLIs are optional.
