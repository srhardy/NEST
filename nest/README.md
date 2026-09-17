# nest — the engine spine

NEST does not re-implement an agent runtime. It **hatches** bots by
wrapping the agent CLIs already on the machine — launch, stream lifecycle
events, collect results, no control plane. A unified provider layer
(`forage`) sits beside it for direct completions.

## The vocabulary (everything is the nest)

| Term | What it is |
|---|---|
| **perch** | a wrapper around one agent CLI (`claude-code`, `codex`, plus builtin `echo`/`fail` stubs) — each harness gets a perch |
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
- `.nest/mcp.json` → harnesses that read it natively get it for free, since
  hatchlings run in the project directory

## The shell — `nest shell`

`nest shell` starts a tiny local server on `http://127.0.0.1:4747`
(`--port` to change) — the one-window app over this same engine:

- **Roster** — every bot as an animated bird (idle / working / waiting /
  stuck / done); each bot owns a colour in the `multi` theme.
- **Ask the nest** — chat driving `brood` dispatch; clicking a roster bot
  chats it directly via `hatch`. Peeps stream live over SSE as loop
  chips — the watch-it-think view.
- **Activity** — the roost's receipts.
- **Settings** — detected perches, power-policy toggles, and the real
  `<home>/settings.md`, editable in place.
- **Themes** — `multi` / `fluro` / `mono`; all dark with white animated
  birds, and toasts/speech bubbles follow the theme too.

- **Onboarding** — first run asks you to name your boss bot, pick its
  abilities, and tick which engines to install; ticking one starts a
  real install streamed as a run (`POST /api/onboarding`,
  `POST /api/perches/install`).
- **Providers** — `providers.toml` declares free/cheap/paid backends
  (`command` or `ollama`); `forage`'s router tries available ones in
  free-first order with failover (`GET /api/state`, `nest providers`,
  `nest ask --provider x`).
- **Routines** — `automations.toml` schedules (5-field cron, UTC) and
  `file-change:` watches run as ordinary dispatches via the patrol;
  armed triggers are listed under settings.
- **Remotes** — the shell watches `~/.nest/remotes/<name>/inbox/*.md`:
  each file is brood-dispatched and the reply written to
  `outbox/<chat|channel>.txt`. The `remotes` package ships stdlib-only
  bridges (telegram / slack / watch) that own the polling side.

Native chrome (taskbar icon, floating desktop birdy, OS notifications)
arrives with the desktop wrapper — the web shell embeds unchanged inside
it.

## The `nest` CLI (Expert-mode passthrough)

```bash
nest perches                          # which harness CLIs are installed
nest agents                           # the roster
nest hatch doc-keeper "sync readme"   # run one bot, peeps stream live
nest brood "update the docs"          # brood picks: routing → boss → fallback
nest roost                            # recent runs (receipts)
nest imprint boss "first look"        # first verified run — 🐣 imprinted
nest ask "prompt"                     # routed provider call (free-first failover)
nest providers                        # declared backends + readiness
```

Flags: `--home` (personal `.nest/` dir, or `NEST_HOME`), `--cwd` (run
directory), `--perch` (pick a harness explicitly).

## Design rules inherited from the content layer

- **Permissions map to harness flags.** On `claude-code`: `allow` →
  `--allowedTools`, `deny` → `--disallowedTools`. `ask` can't prompt in a
  headless run, so it joins the deny list — the prompt still states the
  policy in words, and the receipt shows what was unreachable.
- **`max_steps` is a hard budget.** Step peeps are counted; at the cap →
  kill → `BudgetExceeded` on the receipt.
- **Every attempt is a receipt.** Failed ladder runs land in the roost
  too — the activity log shows the whole fallback trail, not just the
  winner. The same holds for a `clutch`: a spawn failure lands as a
  `SpawnFailed` receipt instead of aborting the batch.
- **Fallback is for failure, not taste.** The brood walks a route's
  fallback chain only when a candidate doesn't finish `Done`.

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
