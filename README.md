# NEST

**One window. A cast of little workers. Built around the agent CLIs you already use.**

NEST turns your machine into a warm home for AI agents — *bots* — that you can see, talk to, and grow. It wraps the command-line tools you already have (Claude, Codex, OpenCode, and friends) in a friendly interface where each bot has a job, a voice, and reusable skills. Point it at a task and NEST dispatches a CLI run; watch its live output and keep reusable skills it learns. Chat transcripts are stored locally under `~/.nest/sessions/` and restore after reload; provider-native context continuity, edit/delete controls, and multi-turn continuation behavior vary by perch and remain limited.

Built on top of Spoke, the shared manifest, projection, and package layer for agent tools.

## Why NEST is awesome

### You don't need to know anything to start
Open NEST and **eggbot** walks you through building a bot. It previews the generated manifest and Spoke projection before building; the current post-build smoke test uses the built-in Echo perch and does not verify a real external engine.

### Bots are characters, not config files
Your roster is a living cast of animated birds on a black canvas — each bot gets its own color, flaps while it works, and tells you when it succeeds or fails. A **watch-it-think** stream shows what your bot is doing as it does it. Dark mode with three themes: fluro, monochrome, and full-color cast.

### The Boss keeps order
Name your own boss agent (default: **Bigbird**) during onboarding. The Boss watches the roost, routes tasks to the right specialist by pattern, and climbs an escalation ladder when a bot fails — assistant to you, boss to the other assistants. Rename it, reshape it, it's yours.

### Free-first, always
When the nest itself needs a model — `nest ask`, `nest feather` — it ranks your configured providers by cost tier (free first, then cheap, then paid) and respects each one's rate limits: spent providers sit out until their window resets. Bots riding real CLIs (Claude, Codex, …) use that CLI's own login; per-bot routing through the provider layer is on the roadmap.

### Bots that learn
When a bot finishes a hard task, `nest feather` distills the run into a reusable skill file — provenance stamped with which bot authored it. Add it to any bot's `skills:` list and its next run starts smarter. The roster literally improves with use.

### Shareable Spoke packages
Spoke can install local or indexed skill/bot packages with `spoke add`. NEST includes a small static registry of starter bots, remote bridge skills, and the Bob council; package commands execute trusted local/Git content, so review a package before installing. This is not a hosted marketplace.

### Keep NEST running in the background
The desktop wrapper can minimize to the taskbar and show notifications. `nest daemon` runs as a foreground process; use an OS service manager if you need it to survive logout or reboot. Services declared in `services.toml` are spawned and health-checked on loopback, with current lifecycle/status limits documented in the project plan.

### Optional remote bridges
The `remotes` package provides local inbox/outbox bridge scripts for Telegram and Slack. They require user-supplied configuration and credentials and run as local processes; they are not a hosted NEST service. The shell UI is one window with tabs, and user-facing settings live in readable local files.

## The tour in one breath

```bash
nest shell              # open the one-window app
nest brood "write a haiku about naps"   # Boss routes it to the right bot
nest feather <run-id>   # distill that run into a reusable skill
nest daemon             # remotes + schedules keep working while you sleep
```

Inside the shell: **Roster** (your cast) · **Chat** (ask the nest or a specific bot) · **+ bot** (eggbot's eight-question walkthrough through agent.toml) · **Activity** (every run, every receipt) · **Settings** (themes, engines, providers, routines — one readable file).

## What's inside

- **`nest/`** — the engine + CLI + local web shell (Rust). Hatches bots through *perches* (thin wrappers over the CLIs you have), streams *peeps*, keeps receipts in the *roost*, routes via the *brood*, schedules via the *patrol*, remembers via skills.
- **`desktop/`** — Tauri chrome: tray icon, close-to-background, draggable birdy, notifications.
- **`spoke/`** — Spoke workspace: `agent.toml` is the manifest, `.nest/` is what it renders. Bots, automations, routing, services — all data, all syncable.
- **`packages/`** — starter packs: `bob-council` (the Boss + eight specialists), `starter-bots`, `remotes`.
- **`docs/`** — `BIG-PLAN.md`: the whole vision, every phase, and the research behind it.

## Status

Phase-by-phase progress lives in `docs/BIG-PLAN.md`. The engine, shell, eggbot, remotes, daemon, desktop chrome, provider router, starter packs, and the learning loop are shipped; hosted services and external triggers are the remaining frontier.
