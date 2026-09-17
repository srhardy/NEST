# Grok Bot research — how x.ai handles bots, and what "eggbot" is

Sources: `docs.x.ai/grok-bot/bots`, `x.ai/bot`, `x.ai/bot/marketplace`,
`x.ai/bot/guides/*`, `x.ai/bot/marketplace/bots/dr-eggbot-v2`
(fetched 2026-09-17).

## The Grok Bot model

A **Bot** is a *durable AI teammate* — not a one-off chat. Anatomy:

- **Profile**: name, title, description, avatar. The *description* is the
  durable system-level instruction ("rules that should remain true" —
  boundaries, ownership, style); the conversation is for task instructions.
- **Conversation**: each Bot owns its own ongoing conversation/context.
- **Memory**: retains stable preferences, facts, and work summaries across
  sessions. Docs warn memory is not an authoritative source.
- **Skills**: packaged capability/knowledge units attached to a Bot.
- **Routines**: triggers that wake a Bot on its own — schedules (cron-like,
  e.g. "weekdays 8:44 PT") and **webhook routines** (external events wake the
  bot; sender keys handled via a `secret-request` flow, never pasted in chat).
- **Plugins/integrations**: tools, MCP servers, sign-ins ("Bots can sign in
  to your tools and use them like you do").
- **Automations/healthchecks**: standing self-check routines (e.g.
  `transcript-healthcheck`, `routine-healthcheck`) that stay quiet unless
  there's something to propose.

## Sidebar & roster UX

- Bots live in the sidebar; **pin** active ones, **hide** without deleting,
  "Show hidden chats" to restore. Hiding does not pause routines.
- Limit: **50 Bots + group chats** per account.
- **Group chats**: several Bots in one thread pass work between themselves —
  user watches handoffs instead of approving every step.
- **Duplicate**: copies profile/settings/skills/routines/avatar, NOT
  conversation history or learned memory (→ one Account Health Bot per region).
- Bots can **suggest or create other Bots** when a job deserves a long-lived
  owner (this is the hook eggbot hangs on).

## Creation flow

1. `New` in sidebar (or `Cmd/Ctrl+N`) → **"Create new agent"**
2. A Bot named "New Agent" opens immediately
3. `Bot actions → Edit Profile` → set name, title, description, avatar
4. Start a conversation with a concrete task

Guidance docs push one design rule hard: **give each Bot ONE clear job** —
a distinct goal/ownership area, tool set, working style, approval boundary,
recurring schedule. "General Helper" is the anti-pattern.

## Sharing: links, templates, marketplace

- **Share link**: recipient sees a preview on x.ai → **"Add to Grok Bot"**
  → a *copy* is created on their account (no history, logins, or computer
  access transfer). Public — anyone with the link sees identity, description,
  skills, routines.
- **Templates** ("recipe, not meal"): `Share as Template` in bot settings
  packages instructions + relevant memories + skills + plugins, **excluding
  secrets, personal memories, custom code/scripts**. `View Details` shows
  exactly what's inside before publishing to **team** or **public**; install
  is one click from the template link. Non-standard plugins (custom MCP
  servers, scripts) do NOT carry over — authors are told to encode setup
  instructions into the template.
- **Marketplace** (`x.ai/bot/marketplace`): sections `Featured`,
  `From Grok Bot Team`, then categories — Engineering, Sales, Marketing,
  Design, Personal. Each card = name, author, a tight job description
  (boundaries stated explicitly: "never emails a candidate without you",
  "nothing sends without your yes"), and an **Add** button.
- **Bot page** (e.g. dr-eggbot-v2): name, "By <author>", longer description,
  bullet details of behaviors/skills, **Import Bot** CTA.
- **Guides**: editorial playbooks ("How I run multiple teams of Grok Bots",
  templates walkthrough) that double as discovery + education.

## dr-eggbot-v2 — anatomy of a bot-builder bot

This is the bot the user wants emulated. From its marketplace listing
(author: Lauren Tan):

- **Job**: designs high-quality Grok Bots. Intake = *a few preference
  questions* → then it calls **`CreateAgent`** to actually build the bot.
- **Design rubric**: lives in a `design-grok-bot` skill it carries.
  - **Coding bots** get the "poteto-mode bar": *one job, unslopped,
    verified* — concise detailed responses, deliberate subagents, unslopped
    prose, simple code, prove-it-works.
  - **Non-coding bots**: *one job, one voice, explicit anti-jobs, no leftover
    tools* — e.g. a mentions scout does not post, a drafter does not send,
    stays quiet when there's nothing to say.
- **Persona**: casual, a little mad-scientist, short lowercase. **Bias to act
  once the job is clear** — doesn't interrogate forever.
- **Does not default to shareable templates** (builds live bots, not recipes).
- **`Make Bot UI` skill**: recipe for custom UIs that wake Grok Bots over a
  webhook — create a webhook routine, handle sender keys via `secret-request`
  (never in chat), host the UI page locally, expose on Tailscale.
- **First-run setup contract** (runs on import): `/setup-pstack`,
  `/create-verification-skill` when a repo exists, then ensures
  `transcript-healthcheck` + `routine-healthcheck` automations exist
  (weekdays 8:44 PT friction scan; Mondays 8:49 PT token/waste audit),
  then offers a fleet routine checkup — *without waiting to be asked*.
- Teaches via the listing itself: the marketplace page doubles as
  documentation of its behavior contract.

## "Poteto mode" distilled (the design bar)

- One job / one voice / explicit anti-jobs
- Laziness, Subtract Before Add, Experience First, Prove It Works
- Deliberate subagents; no leftover tools; unslopped prose
- Verified work — never claim, prove

## What maps onto Command Code

| Grok Bot concept | Command Code equivalent |
|------------------|------------------------|
| Bot profile + description | Persistent agent persona (system prompt + display metadata) |
| Skills | Command Code skills/playbook attachments |
| Routines (cron + webhook) | Scheduled sessions + webhook triggers |
| Plugins / sign-ins | Tool permissions, MCP servers, API keys via secret-request |
| Templates / marketplace | Bot export format + local roster + optional shared registry |
| Group chats | Multi-agent handoffs / parallel sessions |
| `CreateAgent` | Programmatic bot registration in the app |
| `design-grok-bot` skill | eggbot's own rubric/skill bundle |

See `../plan/eggbot-upgrade-plan.md` for the concrete feature design.
