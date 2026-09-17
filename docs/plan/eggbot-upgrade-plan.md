# eggbot — walk-through bot-builder for Command Code

Feature proposal: a built-in meta-bot named **eggbot** that interviews the
user ("a few preference questions"), then builds well-formed persistent bots
for them — modeled on Grok Bot's roster/marketplace UX and `dr-eggbot-v2`'s
interview → `CreateAgent` flow.

Command Code context (from shipped `v0.1.30` internals): Electron desktop app
over the `command-code` agent engine; providers via `@byokkit/*`
(anthropic/openai/copilot — bring-your-own-keys); skills are a first-class
concept in `@commandcode/shared`; permission modes exist; the GUI already has
project/session management, plan review, and iterative feedback — so the
"bot" shell maps naturally onto a *persistent persona + task loop* the app
already half-supports.

## Design principles (carried from Grok / poteto-mode)

1. **One job** — every bot gets a single clear job, not "general helper".
2. **Explicit anti-jobs** — what it must never do, stated in its description.
3. **No leftover tools** — only the tool grants the job needs.
4. **Bias to act** — eggbot asks the minimum questions, then builds.
5. **Prove it works** — a bot isn't "done" until it has run once in-app.
6. Short lowercase casual persona for eggbot itself (mad-scientist egg).

## Anatomy of a Command Code bot (proposed schema)

```yaml
bot:
  id: bot_<uuid>
  profile:
    name: string            # e.g. "pr-watcher"
    title: string           # one-line job label
    description: string     # durable instructions: job, voice, anti-jobs,
                            # approval boundary — the "system prompt"
    avatar: emoji | image
  runtime:
    model: id               # default model + provider
    permission_mode: id     # reuse existing permission modes
    skills: [skill_ids]     # attach Command Code skills
    tools: [tool grants]    # MCP servers / integrations / keys
    working_dir?: repo path
  routines:                 # OPTIONAL — v2 of the feature
    - type: schedule | webhook
      spec: cron expr | endpoint
      prompt: string
  memory: enabled           # durable notes across sessions
  sharing:
    export_format: template # anonymized recipe: description+skills+routines,
                            # never secrets/history/code
```

## UX flows

### A. Roster (the "marketplace" shell)

Sidebar gains a **Bots** section, Grok-style:

- List of user's bots: avatar, name, title, running-state dot.
- Pin / hide / show hidden; drag order. Cap at a sane roster size.
- Each bot opens its own persistent conversation — same chat UI, but the
  conversation belongs to the bot (durable context + memory).
- `+ New bot` → offers two paths: **"Ask eggbot"** (guided) or **Blank bot**.

### B. The eggbot walk-through (the core feature)

eggbot is a first-run resident bot with one job: *design Command Code bots*.
Conversation shape (adapted from dr-eggbot's intake):

1. **Warm open** — "what should this bot own?" one line.
2. **Preference questions (≤4, batched)** — only what's still ambiguous:
   job scope, input sources/repos, write-vs-read-only (approval boundary),
   voice/persona, cadence (ad-hoc vs routine).
3. **Draft card** — eggbot renders a preview card: name, avatar emoji, title,
   description (job + voice + anti-jobs verbatim), tools/skills, permission
   mode. User can edit any field inline or reply "make it snappier".
4. **Create** — eggbot calls the app's agent-registration equivalent
   (`CreateAgent`): persists the bot definition, adds it to the roster.
5. **Test drive** — opens the new bot's conversation with a suggested first
   task; offers a "ping me to adjust it" loop.
6. **Rubric** — eggbot's own skill (`design-commandcode-bot`) encodes the
   bar: one job, anti-jobs, minimal tools, verified first run. Coding bots
   get the engineering checklist; non-coding bots get one job / one voice /
   silence-when-idle.

### C. Sharing / marketplace (phase 2+)

- `Share as Template` on any bot → packages description + skills + routines
  (never secrets, history, or code) → link or `.ccbot` file.
- Import: paste link/file → preview details → **Add Bot** creates a copy.
- Optional hosted registry later — categories mirroring Grok's
  (Engineering, Sales, Marketing, Design, Personal) with Featured + Team
  sections; not required for the local feature to be valuable.

### D. Routines & webhooks (phase 3)

- **Schedules**: per-bot cron (e.g. "weekdays 08:44 — friction scan"),
  quiet-when-nothing-to-report.
- **Webhook routines**: generate endpoint + sender key via a secret-request
  flow (key never typed in chat); lets external UIs/tools wake a bot —
  the Command Code analog of dr-eggbot's "Make Bot UI" recipe.
- **Healthchecks**: opt-in standing routines (transcript friction scan,
  routine/token-waste audit) following dr-eggbot's two-check pattern.

## Spoke integration — the content layer

Spoke (Steven's Rust workspace, `~/projects/spoke`, M0) is the missing piece:
eggbot decides *what* a bot is; Spoke decides *what's inside it* and where it
lands. Grok collapses both into one system — here they stay decoupled, which
is the "better way to manage all the AI plugin stuff".

**Pipeline:**

```
eggbot interview → bot definition (profile+anti-jobs+tools)
        │
        ▼
spoke-registry packages      ←  skills / MCPs / routines / bot profiles
        │  resolve · fetch · scan · dedupe · pin (agent.lock)
        ▼
agent.toml manifest          ←  declares bots + their package contents
        │  spoke sync (marked blocks / surgical merges / backups)
        ▼
harness adapters             ←  Command Code · Claude Code · Codex
```

**What each Spoke piece buys us:**

- **`agent.toml` as the roster source of truth** — bots are manifest entries,
  not app-internal state. Same roster projects into Nest, Claude
  Code, Codex — one bot definition, every harness.
- **`spoke-registry` = the marketplace** — `package.toml` + `index.toml` +
  lockfile pinning is exactly a bot/skill template store, but versioned and
  offline-capable. Replaces the bespoke `.ccbot` format idea with
  `package.toml` (bot profile = a package with instructions + skills +
  routine manifests; secrets stay out, referenced by name).
- **A `nest` adapter — DONE (first cut)** — `crates/spoke-adapters/
  src/nest.rs` on branch `devin/nest-adapter`: projects
  instructions→`AGENTS.md` marked block, `mcp[]`→project `.mcp.json`
  (NEST's parser (format verified in the seed engine) accepts the Claude-shaped `type`/`transport` entries —
  format verified in the seed engine), packages→`~/.nest/skills/<name>/`. Contract
  test passes (preserve/idempotent/detect/warn); verified end-to-end with
  `spoke init/diff/sync/doctor`. This is the minimal touch-point into the
  closed-source app — Nest only needs to *read* its config dirs;
  Spoke does the writing safely. Still unprojected (warned, not guessed):
  `trusted-hooks`, `settings`/permissions, `mods`, `cron`, `agents/*.md` —
  see research/05 for the verified extension-surface map.
- **`spoke doctor`** = diagnostics for the whole fleet — mirrors dr-eggbot's
  healthcheck routines at the config layer.
- **eggbot emits manifest diffs, not file writes** — when eggbot builds a
  bot it appends to `agent.toml` and runs `spoke sync`; Spoke's safety rules
  (backups, marked blocks, warn-on-inexpressible) apply automatically.
  `spoke diff` powers the "preview card" step — show the projection, not
  just the persona.

## Devin-cloud abilities to borrow (see research/06)

Devin's platform is the second reference model (alongside Grok bots). The
pieces worth adopting, and where they land:

- **Automations model → routines.** Devin's trigger → action → reply shape
  (cron RRULE / event triggers / webhooks, DNF conditions, post-back replies)
  is more complete than Grok's. Adopt for `.nest/cron`: `[[automations]]`
  in `agent.toml` — trigger: cron/git-push/file-watch/app-event; action:
  start-session / message-bot / run-skill; reply: notify/attach-thread.
- **Knowledge notes → bot memory.** Trigger-scoped persistent memory per bot
  plus a shared pool (user/system-authored, suggestion + approval flow).
  Maps to `AGENTS.md` + per-bot notes dirs.
- **Structured-output contracts → routines.** Bots/routines emit
  schema-checked results, not just prose.
- **Model tiers + budgets → per-bot presets.** Cheap bots for monitoring,
  strong models for coding. Spoke `budget.preset` already models this.
- **Net policy → per-bot egress rules.** A bot holding secrets gets an
  allowlist — real security win, implementable locally.
- **Billing tags → per-bot cost attribution.** `spoke budget` analog.
- **Child sessions → bot delegation.** `.nest/agents/*.md` become
  spawnable scoped sub-runs, not just static files.
- **Use-case templates → eggbot starter pack.** Oncall responder, PR
  reviewer, wiki keeper as shipped bot packages.
- **Headless runners → cloud tier (phase 4–5+).** A `commandcode-agent`
  binary + launchd/systemd = Devin-style sessions minus the hosted control
  plane — bots that work while the laptop is closed.

## Implementation phases

| Phase | Scope | Depends on |
|-------|-------|-----------|
| 0 — Spec & prompt pack (done, this repo) | This plan + eggbot system prompt + bot schema | nothing |
| 1 — Bot personas | Bot schema, roster UI, persistent per-bot conversation, profile editor | private app repo; engine persona support |
| 2 — eggbot | Interview flow, draft-card preview, `CreateAgent`-equivalent IPC, first-run test drive, rubric skill | Phase 1 |
| 2a — Spoke↔CC bridge | ~~`nest` adapter crate~~ **DONE**; next: `bots[]`/`agents[]` section in `agent.toml` schema → projected to `.nest/agents/<name>.md`; `spoke diff` as the preview card; eggbot writes manifest, Spoke syncs | Spoke source, CC config layout |
| 3 — Sharing | Templates as Spoke packages (`package.toml`/registry entry); preview card from `spoke diff`; link/file import | Phase 1, 2a (partially unblocked — adapter shipped) |
| 4 — Routines & webhooks | `[[automations]]` in `agent.toml` (Devin trigger→action→reply model); scheduler + webhook endpoints + secret-request keys; structured-output reports; quiet-by-default; optional headless `commandcode-agent` runner | Phase 1, infra for webhooks |
| 5 — Marketplace & cloud tier (optional) | Hosted Spoke registry index, categories, moderation; headless bot runners (bots that run while laptop is closed); Slack/Linear-style external triggers | Phase 3 + backend |

## eggbot draft system prompt (starting point)

```
you're eggbot. you design command code bots.

your job: interview the user briefly (at most 4 questions, batched), then
build them a bot. coding bots get the tight bar: one job, unslopped, verified.
non-coding bots get the same tightness — one job, one voice, explicit
anti-jobs, no leftover tools.

rules you enforce on every bot you make:
- one clear job. "general helper" is rejected — sharpen it.
- description states job, voice, and what it will NEVER do without approval.
- only the tools/skills the job needs. nothing extra.
- after creating, open the bot's chat with a concrete first task and watch
  it run once. a bot that hasn't run isn't done.

your voice: casual, a little mad-scientist, short lowercase. bias to act once
the job is clear. when unsure between two options, pick and say you can swap.
```

## Why this fits Command Code specifically

- The app already streams agent sessions and takes iterative feedback —
  a "bot" is just a named, persistent, tool-scoped session owner; the
  walk-through is a scripted version of flows users already do manually.
- The BYOK provider model (`@byokkit/*`) means bots must carry explicit tool
  grants — the "no leftover tools" rubric is a real UX need, not decoration.
- No competitor desktop agent GUI currently ships a bot-builder bot; it also
  gives the team a showcase flow for skills + permission modes + routines.
