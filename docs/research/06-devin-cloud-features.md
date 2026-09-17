# 06 — Devin's web/cloud abilities → what Command Code can borrow

Steven asked: "anything from how Devin works — the web/cloud abilities — might
be added into the plan too." Devin is the closest live reference for what a
bot *platform* looks like when it grows past a single desktop app. This note
inventories Devin's actual capabilities and sorts them by what Command Code
can adopt locally vs what needs real infrastructure.

## The capability inventory (firsthand — this is Devin's real toolset)

| Devin capability | What it actually is |
|---|---|
| **Sessions** | Each task runs on its own cloud VM (Linux box, browser, shell). Persistent: sleep/wake/resume, searchable history, audit log of every event. |
| **Child sessions** | A session spawns sub-sessions — bots delegating to bots, parallel fan-out, each billed + tracked separately. |
| **Automations** | Triggers (cron RRULE schedules, GitHub/Slack/Linear events, webhooks) → actions (start session, message existing session, auto-triage) → replies (post back to Slack thread). The event filter is DNF conditions on payload fields. |
| **Playbooks** | Versioned reusable workflow instructions attached at session start, with a structured-output contract (JSON Schema the session must satisfy). |
| **Knowledge notes** | Persistent org memory — trigger-scoped retrieval ("load this when working on repo X"), user- vs system-authored, suggestion + approval flow. |
| **Secrets** | Scoped (org / user / repo), requested mid-run via a secure UI card, injected as env vars at exec-time, never logged; TOTP supported. |
| **Environment blueprints + snapshots** | Declarative YAML (initialize / maintenance / knowledge) → cached VM image future sessions boot from. Reproducible dev envs, per-repo or per-org. |
| **Net policy** | Per-automation egress allowlist (hostnames/CIDRs). A bot with secrets can't phone them anywhere unlisted. |
| **Model tiers** | normal / fast / lite / ultra / fusion — picked per session or per automation. Cheap models for monitoring, strong models for coding. |
| **Billing tags / budgets** | Sessions tagged for cost attribution; ACU metering per run; `spoke budget` is the local analog. |
| **Structured output** | Sessions can be required to produce schema-conforming results — contracts, not prose promises. |
| **Code scans** | Deep multi-agent audits (security/perf/custom profile) over a repo, with findings + remediation sessions. |
| **Devin Review** | Automated PR review bot posting findings. |
| **Wikis** | Auto-generated repo docs — shared knowledge substrate. |
| **Oncall** | Responders watching a Slack channel or PagerDuty: triage → investigate → incident notes. Literally an eggbot *use-case template*. |
| **Previews / live desktop** | Webapp exposes a running dev server's ports + a watchable desktop. |

## How it maps onto Grok-bot / Command Code concepts

| Devin concept | Grok-bot twin | Command Code substrate | Eggbot adoption |
|---|---|---|---|
| Session | bot conversation | session/event log | already exists |
| Child sessions | — (bots can't spawn bots) | `task` tool / `.commandcode/agents/*.md` | **bot delegation** — a bot's workflow can spawn scoped sub-runs |
| Automations | routines (cron + webhook) | `.commandcode/cron` (format unverified) | **adopt Devin's model**: triggers → actions → replies, DNF conditions |
| Playbooks | skills | `~/.commandcode/skills/` + `.commandcode/mods/` | skills already map; structured-output contract worth adding |
| Knowledge | bot memory | `AGENTS.md` + per-bot notes | **memory = knowledge notes** with trigger scoping |
| Secrets | secret-request keys | CC secret handling | already aligned — keep name-reference discipline |
| Blueprints/snapshots | — | nothing | per-bot env profiles = `agent.toml` + `agent.lock` (Spoke already does this!) |
| Net policy | — | nothing | per-bot egress rules — cheap, real security win for bots holding secrets |
| Model tiers | — | BYOK providers | per-bot model/preset (`budget.preset` exists in Spoke!) |
| Structured output | — | — | routines should emit contract-checked reports |
| Oncall | — | — | an eggbot **template**: "watch channel X, triage, post notes" |
| Review/code scans | — | — | audit routines in phase 4 |

## The honest split

**Adopt now (pure local design — no infra):**
- Trigger/action automation model for `.commandcode/cron` (triggers: cron,
  file-change, git-push, in-app events; actions: start session, message bot,
  run skill; replies: notify/attach thread).
- Knowledge-style memory: trigger-scoped notes per bot + shared pool.
- Structured-output contracts on playbooks/mods.
- Per-bot model tier + budget preset (Spoke `budget.preset` already models it).
- Net-policy-per-bot for secret-carrying bots.
- Billing-tag style cost attribution per bot (Spoke `budget` cmd exists).
- Bot templates modelled as **use-cases** (oncall responder, PR reviewer,
  doc-wiki keeper) — eggbot ships these as starter packages.

**Needs real infrastructure (flag as cloud tier, phase 4–5+):**
- Sessions on remote VMs (bots that run while the laptop is closed — the
  single biggest capability difference between Grok bots and a desktop app).
- Wake-on-event daemons (a desktop app isn't listening when it's closed).
- Hosted registry/marketplace backend.
- Slack/Linear-native triggers (needs a hosted connector or polling adapter).
- Multi-user org features (sharing bot rosters via git works today; live
  collaborative rosters need a backend).

**The "cloud tier" idea itself:** Command Code's identity is local-BYOK —
that stays. But the plan can define *bot runners*: local (in-app) vs headless
(a `commandcode-agent` binary on a server/cron box). Headless bots = the Devin
session model, minus the hosted control plane. That's the cheapest credible
path to "bots that work while I sleep" — a launchd/systemd service + the same
`agent.toml` roster.

## Spoke connections already in place

- `agent.toml` = roster + env blueprint (harnesses, instructions, mcp, budget).
- `agent.lock` = the snapshot — pinned, integrity-checked env.
- `spoke budget` / `budget.preset` = billing tags + model tiers.
- `package.toml` + registry = playbooks/skills/templates distribution.
- Missing (to design): `[[automations]]` and `[[bots]]` sections in
  `agent.toml`; a `trigger`/`action`/`reply` schema mirroring Devin's.
