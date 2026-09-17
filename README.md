# NEST

**One window. A cast of little workers. No terminal required.**

NEST turns your machine into a warm home for AI agents — *bots* — that you can see, talk to, and grow. It wraps the powerful command-line tools you already have (Claude, Codex, OpenCode, and friends) in a friendly interface where each bot is an animated character with a job, a voice, and a memory. Point it at a task and the nest dispatches the right bot; watch it think; keep what it learned.

Built on top of **[Spoke](https://github.com/srhardy/spoke)** — the config/package layer that makes everything a one-click install.

## Why NEST is awesome

### You don't need to know anything to start
Open NEST and **eggbot** walks you through building your first bot with four questions — name, job, personality, what it can touch — then shows you exactly what it will write before a single file is saved.

### Bots are characters, not config files
Your roster is a living cast of animated birds on a black canvas — each bot gets its own color, flaps while it works, and tells you when it succeeds or fails. A **watch-it-think** stream shows what your bot is doing as it does it. Dark mode with three themes: fluro, monochrome, and full-color cast.

### The Boss keeps order
Name your own boss agent (default: **Bigbird**) during onboarding. The Boss watches the roost, routes tasks to the right specialist by pattern, and climbs an escalation ladder when a bot fails — assistant to you, boss to the other assistants. Rename it, reshape it, it's yours.

### Free-first, always
NEST ranks your configured providers by cost tier — free first, then cheap, then paid — and respects each one's rate limits automatically (reserved before dispatch, never a surprise bill). When a provider hits its cap, the next tier picks up. You can always override; you can never be ambushed.

### Bots that learn
When a bot finishes a hard task, `nest feather` distills the run into a reusable skill file — provenance stamped with which bot authored it. Add it to any bot's `skills:` list and its next run starts smarter. The roster literally improves with use.

### Everything is a one-click Spoke package
Remotes (Telegram, Slack, smart watch), provider integrations, council packs, starter bots — every capability arrives as a Spoke package, installed with a single `spoke add`. Add it upstream later; users never manage plumbing.

### It disappears when you don't need it
Minimize to the taskbar and NEST keeps working quietly — themed notification bubbles when a bot finishes, or an animated desktop **birdy** that flaps while your bots work and opens the app when you poke it. Follows your OS theme unless you tell it otherwise.

### Talk to your nest from anywhere
A tiny daemon watches your remotes' inboxes — message it from Telegram or Slack and the reply lands back where you sent it. The GUI is one window with tabs; the backend settings are a single readable `settings.md` you can hand-edit or let the app manage.

## The tour in one breath

```bash
nest shell              # open the one-window app
nest brood "write a haiku about naps"   # Boss routes it to the right bot
nest feather <run-id>   # distill that run into a reusable skill
nest daemon             # remotes + schedules keep working while you sleep
```

Inside the shell: **Roster** (your cast) · **Chat** (ask the nest or a specific bot) · **+ bot** (eggbot's four-question walkthrough) · **Activity** (every run, every receipt) · **Settings** (themes, engines, providers, routines — one readable file).

## What's inside

- **`nest/`** — the engine + CLI + local web shell (Rust). Hatches bots through *perches* (thin wrappers over the CLIs you have), streams *peeps*, keeps receipts in the *roost*, routes via the *brood*, schedules via the *patrol*, remembers via skills.
- **`desktop/`** — Tauri chrome: tray icon, close-to-background, draggable birdy, notifications.
- **`spoke/`** — Spoke workspace: `agent.toml` is the manifest, `.nest/` is what it renders. Bots, automations, routing, services — all data, all syncable.
- **`packages/`** — starter packs: `bob-council` (the Boss + eight specialists), `starter-bots`, `remotes`.
- **`docs/`** — `BIG-PLAN.md`: the whole vision, every phase, and the research behind it.

## Status

Phase-by-phase progress lives in `docs/BIG-PLAN.md`. The engine, shell, eggbot, remotes, daemon, desktop chrome, provider router, starter packs, and the learning loop are shipped; hosted services and external triggers are the remaining frontier.
