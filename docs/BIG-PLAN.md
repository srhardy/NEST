# THE BIG PLAN — NEST: the noob-first AI app

*NEST stands on the shoulders of Command Code (engine seeds), Grok (bot UX), Devin (workspace model) — this is a new work, not a fork.*

*Prepared by Devin, 2026-09-17. This is the single document to read.*
*Deep-dive evidence lives in `research/01`–`06`; this file merges them into
one harmonized plan.*

---

## 1. What we're building (plain English)

**NEST is a new app** — seeded by everything in this workspace — that takes
the power of a CLI AI and wraps it in a modern, one-window, noob-first GUI.
It lives at github.com/srhardy/NEST:

- **Command Code** is the seed for the engine ideas + extension-surface
  formats + its BYOK (bring-your-own-key) local model.
- **Grok's bots** are the seed for animated character agents and the
  marketplace feel.
- **Devin's workspace** is the seed for the one-window, tabbed layout and
  session model.
- **Spoke** is the seed for the content layer (and the reason the app can
  exist safely at all — every file write is backed up and reversible).
- **eggbot** (dr-eggbot on Grok) is the seed for the hand-holding UX: a
  walkthrough that turns "I don't know what a bot is" into "my bot just
  finished its first task." NEST's own walkthrough bot keeps the name eggbot.

The person this is for is Steven-the-noob's archetype: someone who deserves
the power but shouldn't need the docs. Expert mode exists — as an escape
hatch, never a prerequisite.

Three improvement tracks feed the seed:

1. **Fix + slim the seed base** — Command Code's shipped installer is
   broken for everyone (fix ready upstream) and its release carries ~150 MB
   of measurable waste NEST must not inherit.
2. **The bot platform** — named, persistent, animated agents with a job,
   a personality, tools and a schedule, guided by NEST's eggbot.
3. **Spoke as the content layer** — one manifest (`agent.toml`) of
   instructions/skills/MCP/bots, safely projected everywhere; bots become
   manifest entries, not hidden app state.

---

## 2. The product vision — noob-first, experts welcome

Steven's directive: *"GUI Noob, hold my hand, make this easier — the way
Grok made agents into graphical, animated characters, and the way Devin's
webapp feels to use. Expert mode for people who don't need hand-holding,
but the default is the guided path."*

### Design principles (locked)

1. **One window, tabs.** Devin's workspace is the reference: a sidebar for
   the roster, tabs for the thing you're doing (a bot chat, an eggbot
   interview, settings, activity). No nested windows, no terminal required.
2. **Bots are characters, not configs.** Grok's model: every bot has an
   avatar, a name, a job, a personality — animated state (idle / thinking /
   working / done / stuck). The roster is a cast, not a list of YAML keys.
3. **Guided mode is the default.** Every consequential action gets a
   plain-English preview card (powered by `spoke diff` under the hood):
   what it does, what it costs, what it changes, how to undo it. No jargon
   without an "explain" affordance.
4. **Expert mode is a toggle, not a different app.** Same screens; toggling
   reveals the manifest, raw configs, CLI passthrough and skips preview
   steps. The CLI is still the ground truth underneath — the GUI never
   hides that.
5. **Safe by construction.** Undo exists everywhere Spoke touches
   (backups, marked blocks). Destructive actions are never silent and never
   the default.
6. **First run = first success.** Onboarding IS eggbot: open the app, it
   greets you, asks 4 questions, builds your first bot, and shows it
   working — before you know what `agent.toml` is.
7. **Local-first, BYOK.** No hosted lock-in; secrets stay scoped and
   referenced by name. Cloud tier is optional and additive, never required.

### Information architecture (one window)

```
┌──────────────────────────────────────────────────────────────┐
│ [Guided/Expert]                      window chrome + mode     │
├──────────┬───────────────────────────────────────────────────┤
│          │  TAB: Chat — doc-keeper                           │
│  ROSTER  │  TAB: eggbot — building "pr-reviewer"             │
│          │  TAB: Templates (marketplace)                     │
│  🥚 eggbot│  TAB: Activity (runs + receipts)                  │
│  📄 doc-  │                                                   │
│    keeper │  ┌─ preview card (spoke diff) ──────────────────┐│
│  🔍 pr-   │  │ + 3 files change · +1 MCP server · undo: yes ││
│    reviewer│ └──────────────────────────────────────────────┘│
│  📊 monitor│                                                 │
│          │                                                   │
│  [+ bot] │                                                   │
├──────────┴───────────────────────────────────────────────────┤
│ status: doc-keeper working · pr-reviewer idle · budget: ok   │
└──────────────────────────────────────────────────────────────┘
```

- **Roster entries are animated** — avatar + job + live state badge
  (🟢 working / ⚪ idle / 🟡 waiting-for-approval / 🔴 stuck).
- **Every tab is a session** (Devin model): resumable, history, receipts.
- **"+ bot" always opens eggbot** — never a raw form. Expert mode
  additionally offers "new bot from manifest."

### Where each reference product contributes

| From | We take | We deliberately skip |
|---|---|---|
| **Devin webapp** | one window + tabs, sessions, activity log, mode toggle, preview exposure | cloud-hosted requirement, ACU billing model |
| **Grok bots** | animated character agents, marketplace cards, eggbot-style walkthrough, routines | hard 50-bot cap, opaque memory |
| **Command Code** | local engine, BYOK, `.commandcode/*` extension surfaces | terminal-first UX, broken installer path |
| **Spoke** | manifest/lockfile, safe projections, registry, budgets, honest warnings | CLI-only interface (gets wrapped) |

> **eggbot decides what a bot is; Spoke manages what's inside it and where
> it lands; Nest runs it.**

```
        ┌──────────────────────── USER-FACING LAYER ───────────────────────┐
        │  Roster sidebar · eggbot interview · preview card · bot chats    │
        │                          (Nest app)                            │
        └───────────────────────────────┬──────────────────────────────────┘
                                        │ writes manifest, reads diffs
        ┌──────────────────────── CONTENT LAYER ──────────────────────────┐
        │  agent.toml (roster) → agent.lock (snapshot) → spoke-registry   │
        │  (packages = skills/bots/MCPs, versioned + integrity-checked)   │
        │                           (Spoke)                                │
        └───────────────────────────────┬──────────────────────────────────┘
                                        │ safe projections (backups,
                                        │ marked blocks, warnings)
        ┌──────────────────────── DELIVERY LAYER ─────────────────────────┐
        │    Nest · Claude Code · Codex  (harness adapters)               │
        └─────────────────────────────────────────────────────────────────┘
```

Why this layering wins:
- **Spoke writes, the app reads.** The minimal correct touch-point is
  NEST's own config dirs — Spoke handles
  all the risky file surgery with backups and idempotent merges.
- **One roster, every harness.** A bot defined once in `agent.toml` can be
  projected into Nest *and* Claude Code *and* Codex — Spoke's whole
  premise. That's a feature Grok can't offer.
- **The marketplace already exists.** Spoke's `package.toml` + registry +
  lockfile pinning IS a bot/skill template store — versioned, offline-
  capable, secrets-free by construction. No bespoke `.ccbot` format needed.
- **`spoke diff` is the preview card.** Before eggbot builds anything, the
  user sees exactly what will change.

---

## 4. Status — what's already done

| Item | State |
|---|---|
| Release bloat measured & ranked | ✅ `research/01` |
| `install.sh` repo-name bug found + fixed + verified | ✅ `research/02`, patch ready |
| Grok-bot model + dr-eggbot anatomy researched | ✅ `research/03` |
| Spoke downloaded, built, tested (13/13 green) | ✅ `spoke_src/spoke` |
| **`nest` adapter for Spoke — written, tested, verified live** | ✅ branch `devin/nest-adapter` |
| Extension surface mapped from the seed engine (evolves to `.nest/*`) | ✅ `research/05` |
| Devin-cloud feature audit mapped to the plan | ✅ `research/06` |
| `[[bots]]`/`[[automations]]` sections in `agent.toml` | 🔲 next step |
| Access to the private app repo | 🔲 **needs Steven** (see §9) |

---

## 5. Track A — Fix what exists

### 5.1 The installer bug (severity: broken for all users)

`install.sh` greps release JSON for asset URLs containing
`CommandCodeAI/gui` — but releases live on `CommandCodeAI/desktop`. The
matcher can never hit, so macOS/Linux installs always fail with
"No verified package was found in the 20 newest releases."

**Fix (done, verified):** `research/installer-repo-name-fix.patch` — one-word
URL fix plus a stale "Apple silicon only" message correction. Verified via
`COMMANDCODE_INSTALL_DRY_RUN=1` → correctly resolves
`CommandCode-0.1.30-amd64.deb`.
**Action:** patch is ready for upstream; needs a PR on their repo or an
issue report (I couldn't push — no write access).

### 5.2 Debloat (measured, ranked — `research/01`)

v0.1.30 already cut 762→557 MB installed. Remaining, ranked by
effort-to-size ratio:

| Tier | Items | Savings | Effort |
|---|---|---|---|
| **A — config-only** | 55 locale `.pak`s (47 MB) · duplicated nested `node_modules` (75 MB: zod×3, @ai-sdk, @jimp plugin dupes) · foreign-platform `.node` bins (~15 MB) · `.map` files (13 MB) · shipped `.ts` sources (3.9 MB) | **~95–115 MB** | electron-builder excludes + `electronLanguages` |
| **B — judgment calls** | full `@jimp` (32 MB → plugin subset) · `@opentelemetry` (21 MB → lazy init) · `react-reconciler` **dev build** shipped | ~55–70 MB | needs source access to confirm usage |
| **C — hygiene** | `asar` packaging · per-platform pruning of `@crosscopy`/`@parcel/watcher`/`node-pty` | smaller download, faster install | config |

**Also a real bug hiding in B:** shipping `react-reconciler.development.js`
means a dev build made it into production — worth a packaging audit pass.

**Action:** needs the private repo's `electron-builder.yml` to apply. All
Tier A items are pure config; Tier B needs a usage audit in the source.

---

## 6. Track B — Spoke: the plumbing (substantially DONE)

Spoke M0 shipped with: `init sync diff doctor add remove list why dedupe
event hooks budget`. Design rules that make it trustworthy: never-clobber
(marked blocks + surgical merges), backup before every write, warn-don't-
drop, idempotent, no telemetry, no code execution.

**New this session:** `crates/spoke-adapters/src/command_code.rs` —
Nest is now a first-class Spoke harness:

| `agent.toml` declares | Lands in Nest as |
|---|---|
| `instructions.extra` | marked block in `AGENTS.md` (its native memory file) |
| `mcp[]` (command xor url) | `mcpServers.<name>` in project `.mcp.json` |
| packages (skills) | files in `~/.nest/skills/<name>/` |
| (not yet) hooks/permissions/mods/cron | warned as gaps — formats unverified |

Verified end-to-end: `spoke init → diff → sync → doctor` against a fake home
— detection, surgical writes, idempotent second run, capability-gap
warnings all correct.

**SHIPPED (PR #2):** all three "still to build" items below are now real —
1. `[[bots]]` in `agent.toml` → `nest` adapter renders `.nest/agents/<name>.md`
   (YAML frontmatter + persona; validated: kebab names, non-empty job, unique).
2. `[[automations]]` → `.nest/automations.toml` (`schedule` XOR `event`,
   `prompt`, `bot`, `enabled`) — NEST's own format, defined here.
3. `package.toml` `[[agents]]` → packages ship bot definitions into
   `~/.nest/agents/<name>/`, recorded in `agent.lock`, removed by `spoke remove`.
Plus `spoke bot list` / `spoke bot add <name> --job "…"` CLI sugar, `doctor`
counts, and a git-managed `.devin/blueprint.yaml` for the Rust toolchain.
Remaining known gap: removing a `[[bots]]` entry doesn't delete the rendered
`.nest/agents/<name>.md` yet (warned, not silent).

**SHIPPED (PR #3) — "Bob the Boss" phase:** BobTheBuilder folded into the
schema and packaged as NEST's first council —
1. `bots[].permissions` (per-tool `allow`/`ask`/`deny`), `bots[].mode`
   (`primary`/`subagent` — subagents spool, return, auto-clean), `bots[].boss`
   (max one orchestrator), `bots[].max_steps` — all rendered into the
   `.nest/agents/<name>.md` frontmatter, mirroring Bob's agent format.
2. `[[routing]]` → `.nest/routing.toml` — the Boss's dispatch table
   (patterns → bot + `fallback` escalation ladder), modeled on Bob's
   auto-detection routing.
3. `[[services]]` → `.nest/services.toml` — declared daemons
   (name/port/health/start) the app can boot and health-check, modeled on
   Bob's `orchestrate.sh` service table.
4. `packages/bob-council/` — a path-sourced package shipping BobTheBuilder's
   8 specialist agents (converted to NEST format, permission maps intact)
   + 6 skills → `spoke add packages/bob-council` installs the council into
   `~/.nest/`. Scan warnings (network refs, host script) surface as designed.

---

## 7. Track C — Eggbot + the bot platform

### 7.1 What a Nest bot IS (final schema)

Harmonized from Grok's model + the seed engine's proven surfaces + Devin's extras:

```yaml
# a bot = an entry in agent.toml, projected to .nest/agents/<name>.md
bot:
  name: doc-keeper            # one job, lowercase-friendly
  title: "Doc Keeper"
  job: "Keep project docs in sync with code changes"   # THE one job
  voice: "precise, dry, minimal"
  anti_jobs:                  # what it will NEVER do without approval
    - "edit code files"
    - "commit without asking"
  skills: [markdown-tools]    # only what the job needs — no leftover tools
  mcp: [github]               # tool grants, explicit
  model_tier: fast            # Devin-style: cheap bots monitor, strong bots build
  budget: daily-50k           # Spoke budget preset
  memory: project             # knowledge-note scope
  routines: []                # [[automations]] — phase 4
  egress: [github.com]        # per-bot net allowlist (Devin net-policy idea)
```

The "poteto-mode" bar eggbot enforces on every bot it builds: **one job ·
unslopped · verified working · explicit anti-jobs · no leftover tools.**

### 7.2 The three core UX flows

**Roster** — sidebar section listing bots (name · job · status). Pin/hide,
per-bot chat, "Add bot" → eggbot. (`dr-eggbot` reference: card = name +
author + job + Add button.)

**The interview** — eggbot asks ≤4 batched questions (what's the job? what
should it never touch? which tools? how chatty?), then shows the **preview
card = `spoke diff`** (persona + exactly which files/servers it will add),
then builds: appends to `agent.toml`, runs `spoke sync`, opens the bot's
chat with a concrete first task and **watches it run once** — a bot that
hasn't run isn't done.

**Routines** — scheduled/webhook-triggered runs. Adopt Devin's
trigger→action→reply model (richer than Grok's):
`trigger` = cron RRULE / git-push / file-watch / app-event;
`action` = start-session / message-bot / run-skill;
`reply` = notify / attach-thread / post-report.
Quiet by default; structured-output contract on reports.

### 7.3 eggbot's draft system prompt (ready to use)

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

### 7.4 Devin-style abilities folded in (research/06)

Adopt now, locally: automation trigger/action/reply model · trigger-scoped
bot memory (knowledge notes) · structured-output contracts · model tiers +
budgets · per-bot egress allowlists · per-bot cost attribution · bot
delegation (agents spawning scoped sub-runs) · starter templates (oncall
responder, PR reviewer, wiki keeper — proven Devin use-cases).

Cloud tier, phase 4–5+: **headless `nest-agent` runner** via
launchd/systemd — Devin's session model without a hosted control plane —
plus a hosted registry and external Slack/Linear-style triggers.

---

## 8. The phase roadmap (priorities after Steven's noob-first directive)

| Phase | Deliverable | Depends on | Effort (Devin sessions) |
|---|---|---|---|
| **0 — Spec & research** | ✅ this plan, all research docs, adapter | — | done |
| **1 — Plumbing finish** | `[[bots]]` + `[[automations]]` in `agent.toml` schema; adapter writes `.nest/agents/*.md`; `package.toml` carries bot defs; CLI `bot list/add` sugar | nothing — pure Spoke repo work | ~1 session |
| **1a — Installer upstream** | patch lands on CommandCodeAI/desktop (or issue filed) | their repo access or issue | minutes |
| **2 — The Shell (flagship)** | one-window app: tab strip, sidebar roster of **animated bot characters** (idle/thinking/working/stuck states), Guided/Expert mode toggle, first-run onboarding | **new repo (Steven is creating)** + app engine access | 1–2 sessions after access |
| **2a — eggbot in the shell** | the walkthrough: 4-question interview → `spoke diff` preview card → build → test-drive → hand-off to roster; rubric skill enforcing the one-job bar | phase 1 + 2 | 1 session after access |
| **3 — Sharing** | bots as Spoke packages; link/file import; starter template pack (oncall responder, PR reviewer, wiki keeper) | phase 1 | ~1 session |
| **4 — Routines** | scheduler + webhooks in-app; `[[automations]]`→`.nest/cron` once format verified; quiet reporting + receipts | phase 2; cron format | 1–2 sessions |
| **5 — Cloud tier (optional)** | headless `nest-agent` binary; hosted registry; external triggers | phase 3 + backend | 2–3 sessions |
| **B — Debloat (parallel)** | Tier A config (~95–115 MB) + installer fix + packaging audit | private repo's build config | <1 session |
| **B2 — Debloat tier B** | jimp/otel/reconciler audit + fixes | source read access | <1 session |

**Critical path:** Phase 1 (Spoke schema) needs nothing but your repo — I
can start it the moment the GitHub repo is connected. Phase 2 (the shell)
needs the new project repo + the app engine. The GUI is now Phase 2, not an
afterthought — the noob-first shell IS the product.

---

## 9. What YOU need to do (plain-English checklist)

These are in order. Each is a few minutes. Copy-paste commands are provided.

### Step 1 — Save this workspace to your Mac
Download the attachments from this chat, then in Terminal:
```bash
mkdir -p ~/projects/nest
# unzip the files you downloaded into ~/projects/nest/
unzip -o ~/Downloads/spoke-src-adapter.zip -d ~/projects/nest/
```
(The zip is your whole Spoke project INCLUDING the `nest` adapter + git
history — it can replace your existing `~/projects/spoke` folder.)

### Step 2 — Put NEST + Spoke on GitHub
You made `github.com/srhardy/NEST` — now put the code in it:
```bash
cd ~
git clone https://github.com/srhardy/NEST.git && cd NEST
cp -R ~/projects/nest/spoke_src/spoke ./spoke   # spoke lives inside NEST
mkdir -p docs && cp ~/Downloads/BIG-PLAN.md docs/
git add -A && git commit -m "seed: spoke content layer + nest adapter + BIG-PLAN" && git push
```
(Spoke inside the NEST repo = one connection, one home. If you'd rather
keep `srhardy/spoke` separate that works too — push it there instead.)

### Step 3 — Connect the repo to Devin
Go to https://app.devin.ai/settings/repositories and add `srhardy/NEST`.
That's it — from then on I clone it, work on branches, and open PRs for
you to review. This is the "GitHub + Devin play nicely" step: one click
there, and everything after flows through pull requests you can read.

### Step 4 — Engine decision
NEST needs an agent engine. Options, easiest first:
- **Build NEST's own** inside the NEST repo — Rust or TypeScript core that
  reads `.nest/`, `AGENTS.md`, `mcp.json` (the formats are already spec'd;
  BYOK via the same provider pattern). Most work, most ownership.
- **Reuse the seed engine** — if you can get source access to Command
  Code's engine, NEST wraps it. Tell me who owns it / how you got the app
  and I'll help you figure out the path.
- **Start thin** — the shell + Spoke + a simple chat loop first (call any
  provider API directly), upgrade the engine later. *(Rec: start thin —
  it's the fastest route to a real window on screen.)*

### Step 5 — Send the installer fix upstream (optional, good deed)
Apply `research/installer-repo-name-fix.patch` and open a PR on
`CommandCodeAI/desktop` — or file an issue with the patch attached. Their
installer is currently broken for every macOS/Linux user.

### Step 6 — Make three decisions (reply with your picks)
1. **Bot scope:** roster per-user-global, per-project, or global-with-
   project-pin? *(Recommendation: global roster, optional project pin —
   matches how skills already have both `~/` and project tiers.)*
2. **Bot cap:** hard-cap at 50 like Grok, or unlimited? *(Rec: unlimited —
   local bots don't share a quota.)*
3. **Cloud tier later:** okay to design toward an optional headless runner,
   or strictly local-only forever? *(Rec: design for optional headless —
   it's the only honest way to "bots that work while the laptop is closed.")*

### Then you're done — I take it from there
With Steps 2–3 done I can keep building Phase 1 (Spoke `[[bots]]` schema +
agent projection) inside NEST immediately. With Step 4's call made I start
Phase 2 — the shell window with the animated roster and eggbot.
Everything else in the plan is mine.

---

## 10. Glossary (for the non-expert)

- **Harness** — an AI app Spoke writes config into (Nest, Claude
  Code, Codex are harnesses).
- **Manifest (`agent.toml`)** — one file listing what a project wants:
  instructions, MCP servers, skills, (soon) bots.
- **Lockfile (`agent.lock`)** — the pinned record of what got installed +
  checksums, so every install is identical and auditable.
- **Adapter** — the small translator that turns manifest entries into one
  harness's file formats. The `nest` adapter now exists.
- **MCP** — the standard plug format AI tools use for external tools/data.
- **Bot / agent file (`.nest/agents/*.md`)** — Nest's native
  sub-agent definition; our bots will be written there.
- **Routine / automation** — a bot doing work on a schedule or when an
  event fires (like a cron job with manners).
- **Skill (`SKILL.md`)** — a packaged how-to the AI loads when relevant.
- **Marked block** — `<!-- spoke:begin/end -->` regions Spoke owns inside a
  file; everything outside is untouched. How nothing gets clobbered.
- **eggbot / poteto-mode** — the walk-through bot-builder; its quality bar:
  one job, unslopped, verified, explicit anti-jobs, no leftover tools.
- **Headless runner** — the app engine running on a server with no screen;
  how bots could work while your Mac is shut.

## 11. Honest risks

- **Private repo is the gate.** Phases 2+ stall without it; the adapter and
  schema work don't. *Mitigation: Step 4.*
- **`.nest/cron`/`trusted-hooks`/`settings` formats in design, unverified.** We
  warn rather than guess; formats need the source or live experimentation.
- **Debloat Tier B needs usage audit.** Can't confirm jimp/otel/reconciler
  usage without source — Tier A is safe regardless.
- **Installer fix needs upstream acceptance** (or fork the installer).
- **Scope discipline.** The plan is big; the phases keep each chunk
  shippable on its own. Phase 1 alone makes Spoke→Nest real.

---

*Appendix — doc map: `research/01` bloat · `02` installer · `03` Grok ·
`04` Spoke overview · `05` CC extension surface · `06` Devin cloud ·
`plan/resources-needed.md` checklist · `spoke_src/spoke` the code.*
