# nest — the engine spine

NEST does not re-implement an agent runtime. It **hatches** bots by
wrapping the agent CLIs already on the machine — launch, stream lifecycle
events, collect results, no control plane. A unified provider layer
(`forage`) sits beside it for direct completions.

## The vocabulary (everything is the nest)

| Term | What it is |
|---|---|
| **perch** | a wrapper around one agent CLI (`claude-code`, `codex`, `opencode`, `pi`, plus builtin `echo`/`fail` stubs) — each harness gets a perch |
| **hatchling** | a running bot instance — an egg hatches into a bot |
| **peep** | lifecycle events streamed off a hatchling (`spawned`, `output`, `step`, `ask`, `budget_exceeded`, `done`, `failed`) — the single stream the shell and the future watch-it-think dashboard render |
| **roost** | where finished runs rest: JSON receipts in `~/.nest/roost/` — the activity log |
| **brood** | the dispatch council: routes a task to the right bot via `.nest/routing.toml`, falls back up the ladder when one fails, and defers to the boss for everything unmatched |
| **clutch** | a batch of hatchlings run in parallel (the council pattern's primitive) |
| **forage** | the provider layer — prompt in, completion out (echo stub + any local CLI today; the free-tier failover proxy plugs in later) |
| **imprint** | a bot's first verified run — a bot that hasn't run isn't done |

## What it reads

Everything Spoke projects, the engine consumes:

- `.nest/agents/<name>.md` (project) and `~/.nest/agents/<name>/` (packages)
  → the roster (`AgentSpec`: job, mode, boss, max_steps, permissions, persona)
- `.nest/routing.toml` → the brood's dispatch table
- `.nest/services.toml` → declared daemons (surfaced by the shell, not yet booted)
- `.nest/mcp.json` → projected by Spoke from `mcp[]` declarations, but no
  harness reads it from `.nest/` today (nest itself doesn't consume it yet)

## The shell — `nest shell`

`nest shell` starts a tiny local server on `http://127.0.0.1:4747`
(`--port` to change) — the one-window app over this same engine:

- **Roster** — every bot as an animated bird (idle / thinking / working /
  waiting / stuck / done); each bot owns a colour in the `multi` theme,
  and every roster bot gets its own run-history tab (task, chat, test-drive).
- **Ask the nest** — chat driving `brood` dispatch; clicking a roster bot
  chats it directly via `hatch`. Peeps stream live over SSE as loop
  chips — the watch-it-think view.
- **Activity** — the roost's receipts.
- **Settings** — detected perches, power-policy toggles, and the real
  `<home>/settings.md`, editable in place.
- **Expert** — the raw truth, verbatim: `agent.toml` editable in place
  (validated before it lands), everything else read-only, plus a
  read-only CLI passthrough (`POST /api/cli`: agents, perches,
  providers, roost).
- **Themes** — `multi` / `fluro` / `mono`; all dark with white animated
  birds, and toasts/speech bubbles follow the theme too.

- **Onboarding** — first run asks you to name your boss bot, pick its
  abilities, and tick which engines to install; ticking one starts a
  real install streamed as a run (`POST /api/onboarding`,
  `POST /api/perches/install`).
- **Providers** — `providers.toml` declares free/cheap/paid backends
  (`command`, `ollama`, or `openai` for any OpenAI-compatible endpoint:
  `base_url` + `model`, key via `api_key_env`); `forage`'s router tries
  available ones in free-first order with failover. `limit`/`window`
  ("5h", "24h", "7d") adds a CodexBar-style cap per provider — spent
  windows sit out until reset, tracked in `~/.nest/usage/` and shown
  on the settings tab (`GET /api/state`, `nest providers`,
  `nest ask --provider x`).
- **Routines** — `automations.toml` schedules (5-field cron, UTC),
  `file-change:` watches, `git-push` (new HEAD commits), and `app:<name>`
  pings (`<home>/events/<name>.ping`, consumed on fire) run as ordinary
  dispatches via the patrol; `action: brood` forces the council,
  `reply: quiet` skips outbox writes; `POST /api/duties/<name>/fire`
  fires one now (the settings button and external webhooks share it);
  armed triggers list under settings.
- **Daemon** — `nest daemon` boots declared `[[service]]`s (health/port
  check, `start` when down, warnings never fatal) then runs the watchers
  (patrol + remotes) with no HTTP at all: the launchd/systemd unit for
  "bots that work while the lid is closed"; receipts still land in the roost.
- **Sharing** — settings marketplace exports any roster bot as a Spoke
  package dir (`POST /api/share` → `<home>/shared/<bot>-bot/` with
  package.toml + agents md); `spoke add <that dir>` installs it anywhere.
- **Feather** — `nest feather <run-id>` distills a finished run into a
  reusable `~/.nest/skills/<name>/SKILL.md` — provenance frontmatter
  (`authored-by`, `run`) records which bot earned it; another bot picks
  it up by listing `skills: [name]` in its frontmatter. The roster gets
  smarter with use. Opt in to `auto_feather: true` in settings.md and
  hard-won/failed runs distill themselves; the settings tab lists every
  learned skill, and `agent.lock` records each feather's provenance.
- **Remotes** — the shell watches `~/.nest/remotes/<name>/inbox/*.md`:
  each file is brood-dispatched and the reply written to
  `outbox/<chat|channel>.txt`. The `remotes` package ships stdlib-only
  bridges (telegram / slack / watch) that own the polling side.

Native chrome lives in `desktop/` (Tauri): the same shell embedded
in-process — taskbar icon with a bird menu, close-to-background, a
floating always-on-top **birdy** mascot (drag to move, click to reopen,
flaps while bots work), and OS notifications on new receipts.

## The `nest` CLI (Expert-mode passthrough)

```bash
nest perches                          # which harness CLIs are installed
nest agents                           # the roster
nest hatch doc-keeper "sync readme"   # run one bot, peeps stream live
nest brood "update the docs"          # brood picks: routing → boss → fallback
nest clutch boss "task one" "task two" # one bot, many tasks, in parallel
nest roost                            # recent runs (receipts)
nest imprint boss "first look"        # first verified run — 🐣 imprinted
nest ask "prompt"                     # routed provider call (free-first failover)
nest providers                        # declared backends + readiness
nest feather <run-id> [--name slug]   # distill a run into a skill
```

Flags: `--home` (personal `.nest/` dir, or `NEST_HOME`), `--cwd` (run
directory), `--perch` (pick a harness explicitly).

## Design rules inherited from the content layer

- **Permissions map to harness flags.** On `claude-code`: `allow` →
  `--allowedTools`, `deny` → `--disallowedTools`. On `codex`: a denied or
  asked write-ish tool (write/edit/bash/shell/exec/apply/patch/powershell)
  drops the run to `--sandbox read-only` (else `workspace-write`), and
  approval is always `--ask-for-approval never` — Codex has no per-tool
  flags, only the two global dials. On `opencode`: permissions ride
  `OPENCODE_PERMISSION` (inlined JSON `permission` config; `write` folds
  into `edit`). On `pi`: asked-or-denied tools join `--exclude-tools`.
  `ask` can't prompt in a headless run anywhere, so it always joins the
  deny side — the prompt still states the policy in words, and the receipt
  shows what was unreachable.
- **`max_steps` is a hard budget.** Step peeps — harness work units —
  are counted; at the cap → kill → `BudgetExceeded` on the receipt.
- **`max_seconds` is the wall-clock budget.** Some perches never emit
  Step peeps (echo, opencode), so `max_steps` can't trip on them —
  the wall clock can. Default 600s; the kill emits a `TimedOut` peep
  and lands as `BudgetExceeded`. Kills take the whole process group
  (unix), so grandchildren can't hold the pipes open past the kill.
- **Every run leaves a peep tape.** `<roost>/<id>.peeps.jsonl` is the
  full event stream (one JSON per line, `ms` stamped) behind the
  receipt's summary — replay it for audit, debugging, or feathering.
- **Every attempt is a receipt.** Failed ladder runs land in the roost
  too — the activity log shows the whole fallback trail, not just the
  winner. The same holds for a `clutch`: a spawn failure lands as a
  `SpawnFailed` receipt instead of aborting the batch.
- **Fallback is for failure, not taste.** The brood walks a route's
  fallback chain only when a candidate doesn't finish `Done` — and it
  stops early on `failure_kind` `budget`/`denied`: a stopped run was a
  real action, and the next bot wouldn't be a retry.

## Layout

```
nest/
  src/
    spec.rs       # AgentSpec + .nest/ loaders
    perch.rs      # Perch trait + presets + detect
    hatchling.rs  # spawn, peep stream, step budget, receipts
    peep.rs       # the event enum
    roost.rs      # receipt store
    brood.rs      # dispatch council
    clutch.rs     # parallel runs
    forage.rs     # provider layer
    main.rs       # the nest CLI
  tests/spine_test.rs   # e2e against the builtin perches
```
