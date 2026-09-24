# 08 — Paperclip deep dive and fold plan

What NEST can take from [paperclipai/paperclip](https://github.com/paperclipai/paperclip),
where it lands in the spoke backbone, and what to deliberately leave behind.

## Verdict

Paperclip is a *company runtime*: Postgres, ~130 tables, ~200 services, adapters that
invoke agents through a heartbeat contract, humans approving in an inbox. NEST is a
*bot home*: a manifest, a projection, a roster, one binary, no database. The two
architectures do not merge — but paperclip published its package format as a
vendor-neutral spec, and its catalog content is file-based markdown. **The fold is a
Spoke problem**: teach `spoke` to read their format and project it, harvest their
catalog as packages, and borrow their *file shapes* (wakeup spools, approval queues,
task frontmatter) where NEST's runtime has a real gap. Invoke their code; port
nothing.

## The wheel: `agentcompanies/v1`

`docs/companies/companies-spec.md` — a 600-line normative spec, explicitly written
"to be usable by any agent-company runtime, not only Paperclip":

- Package kinds keyed by one canonical file: `COMPANY.md`, `TEAM.md`, `AGENTS.md`,
  `PROJECT.md`, `TASK.md`, `SKILL.md` (SKILL.md stays owned by the Agent Skills spec).
- Folder convention over wiring: `agents/<slug>/AGENTS.md`, `teams/<slug>/TEAM.md`,
  `projects/<slug>/tasks/<slug>/TASK.md`, `skills/<slug>/SKILL.md`.
- YAML frontmatter: `schema: agentcompanies/v1`, `slug`, `name`, `description`,
  `includes[]` (the package graph, local or pinned-URL refs), `manager`,
  `reportsTo`, `assignee`, `project`, `recurring: true`, `requirements.secrets[]`,
  `sources[]` (pinned commit + sha256 + attribution).
- Vendor fidelity lives in sidecars (`.paperclip.yaml`); importers ignore unknown
  extensions.
- No central registry required — a git repo is a valid package container.

This is the same philosophy as spoke (`manifest → projection → lockfile`, local
paths, git-backed). A conforming reader is a few hundred lines, not a port.

## Shape mismatch (why we project, not adopt)

| Axis | Paperclip | NEST |
|---|---|---|
| Truth | Postgres rows (agents, issues, runs as tables) | `agent.toml` / `.nest/*.md` files rendered by spoke |
| Runtime | Node server + heartbeat adapters (`packages/adapters/*-local`) | `brood`/`hatchling` spawn on `perch` CLIs |
| Tasks | `issues` table: hierarchy, checkout lock, workflow status, monitors | Throwaway prompt strings in `brood`/`clutch` |
| Scheduling | `agent_wakeup_requests` queue: coalescing, idempotency, retry windows | `patrol` fires duties immediately; no queue |
| Budgets | `budget_policies` + `cost_events` + hard stops (billed_cents) | `budget` field rendered into prompt (advisory) |
| Approvals | `approvals` + `decision_queues` + human inbox | `ToolPolicy::Ask` exists, has nowhere to go |
| Distribution | `agentcompanies/v1` markdown packages + catalogs | spoke packages (`package.toml`, agents/, skills/) |
| Secrets | `company_secrets` + versions + bindings + access events | `$VAR` env references in manifest |
| Channels | `chat_channels` bound to providers (discord/github/telegram/teams) | file-spool `remotes/` inbox/outbox + webhooks |
| UI | React app, multi-tenant company dashboard | `shell` local UI on 127.0.0.1:4747 |

## Gap map — what paperclip has that NEST lacks

Each row cites the paperclip source so the shape is copied from real code, then
states the NEST landing zone. Verdicts: **fold** = implement in spoke/nest;
**invoke** = reuse their running code; **skip** = out of scope.

| # | Paperclip capability | Paperclip source | NEST state | Verdict |
|---|---|---|---|---|
| 1 | `agentcompanies/v1` package format | `docs/companies/companies-spec.md` | spoke reads `package.toml` only | **fold** — spoke reader |
| 2 | Team/agent catalog content | `packages/teams-catalog/catalog/{bundled,optional}` (TEAM.md + agents/<n>/AGENTS.md + projects/tasks) | 3 starter bots, bob-council pack | **fold** — convert to spoke packages |
| 3 | Skill catalog content | `packages/skills-catalog/skills/**/SKILL.md` (~20 dirs: engineering-*, product-*, docs, quality, paperclip-operations) | `~/.nest/skills/`, index in state | **fold** — convert to spoke skill packages |
| 4 | Work units (issues) | `schema/issues.ts`: `parentId`, `goalId`, `assigneeAgentId`, `checkoutRunId`, `executionLockedAt`, `statusVersion`, `monitorNextCheckAt`, `originKind/originId`, `blockedTransitionAt` | none — tasks are strings | **fold** — `.nest/tasks/<slug>.md` |
| 5 | Wakeup queue | `schema/agent_wakeup_requests.ts`: `source`, `reason`, `payload`, `coalescedCount`, `idempotencyKey`, `requestedBy*` | patrol dispatches instantly | **fold** — `.nest/wakeups/*.json` spool |
| 6 | Run event stream | `schema/heartbeat_run_events.ts` (typed append-only events per run) | peeps in-memory; transcript persisted | **fold** — `roost/<run>/events.jsonl` |
| 7 | Real budgets | `schema/budget_policies.ts` + `cost_events` + `provider_trace_records`: scope, window, amount, warn%, hardStop | advisory prompt text | **fold** — enforce in `hatchling` against `~/.nest/usage/` |
| 8 | Approval queue | `schema/approvals.ts` + `decision_queues` + `decision_training_examples` | `ask` permission has no destination | **fold** — `.nest/approvals/` + shell inbox |
| 9 | Org chart | `agents.reportsTo`, `capabilities` text, `pauseReason` | flat roster + `boss` flag + routing.toml | **fold** — `reports_to`/`capabilities` frontmatter |
| 10 | Routine runtime | `schema/routines.ts`: `concurrencyPolicy` (coalesce_if_active), `catchUpPolicy` (skip_missed), `variables`, `env`, `assigneeAgentId`, revisions, `routineTriggers`, `routineRuns` | patrol: cron/file/git/app triggers, no concurrency model | **fold** — extend patrol duty frontmatter |
| 11 | Recurring tasks in packages | `TASK.md` `recurring: true` → routine on import | n/a | **fold** — maps to patrol `[trigger]` entries |
| 12 | Plan decomposition | `schema/issue_plan_decompositions.ts` → `childIssueIds` | none (brood routes, doesn't decompose) | **fold** — later; `nest plan <task>` |
| 13 | Completion contracts | `schema/completion_contracts.ts`: versioned `contractJson`, `completionAuthority`, `risk`, `supersedesContractId` | none | **fold** — ties to bob-council verifier |
| 14 | Work products | `schema/issue_work_products.ts`: typed deliverables, `reviewState`, `isPrimary` | receipts only | **fold** — `tasks/<slug>/artifacts/` |
| 15 | Task-bound sessions | `schema/agent_task_sessions.ts` (agent+taskKey session continuity) | sessions keyed by run | **fold** — session key = bot+task |
| 16 | Per-run workspace | `server/src/worktree-seed-manifest.ts`, execution-workspace leases; adapters' "no-remote-git contract" (`packages/adapters/AUTHORING.md`) | bots share cwd | **fold** — `git worktree` per run (optional) |
| 17 | Channel bindings | `schema/chat_channels.ts`: `connectionId`, `provider`, `assignedAgentId`, `publicationMode`, `externalExecutionPolicy` | remotes = file spool | **fold** — `channels.toml` binding layer |
| 18 | Provider integrations | `server/src/services/` discord, github (reviews/checks), telegram, teams | webhooks only | **invoke** — run their adapters/bridges, don't port |
| 19 | Secrets vault | `schema/company_secrets*.ts`: versions, bindings, access events | `$ENV` refs | **fold** — `.nest/secrets.toml` manifest of names + `secrets/` dir, file-mode guarded |
| 20 | Skill studio | `schema/company_skills*` + `Versions/Stars/Comments/TestInputs/TestRunTemplates/TestRuns` | feather writes SKILL.md + index | **fold** — skill `versions/` dir + `nest skill test` |
| 21 | Whole-roster export | `services/company-portability.ts` (export/import, collision handling, preview) | `nest share` per-bot | **fold** — `nest export` emits `agentcompanies` COMPANY.md package |
| 22 | Agent adapters (runnable) | `packages/adapters/*-local` + `ADAPTER_SDK` + `AUTHORING.md` | perches: thin CLI wrappers | **invoke** — adapter contract is a superset of perch; a perch can shell out to `paperclip` adapter processes where useful |
| 23 | Attention inbox | `schema/attention*` / inbox UI patterns | run history tabs | **fold** — shell "inbox" tab (approvals + blocked + failed) |
| 24 | Pipelines | `schema/pipelines.ts` + `pipeline_cases`: named workflows, `enforceTransitions` | none | **fold** — later; `[[pipeline]]` manifest section |
| 25 | Announcements | `schema/announcements*` | none | **fold** — `nest announce` → prepended to next prompt for all bots |
| 26 | Status summary | `services/status-cards`/`summary-slots` | `/api/state` aggregate | **fold** — roost roll-up endpoint |
| 27 | Activity log | `activity_log` table | runs only | **fold** — append to `events.jsonl` |
| 28 | Plugins system | `packages/plugins/*` (entities, jobs, webhooks, own DB) | none | **skip** — spoke packages are the extension point |
| 29 | Multi-tenancy, auth modes, EE tiers | `doc/GOAL.md` runtime modes, session auth | local-trusted by definition | **skip** |
| 30 | Postgres/Drizzle, telemetry pipeline | `packages/db`, observability | files | **skip** |
| 31 | Container sandbox providers | `packages/plugins/sandbox-providers/{e2b,daytona,createos}` | none | **skip** for now; revisit if a perch needs isolation |

## The fold plan

Every phase is file-first: works offline, on a bare checkout, on Windows/Linux/macOS
(no POSIX assumptions: paths via `std::path`, no daemons, no signals). All land as
spoke-core types + CLI surface + (where shown) shell endpoints. Estimates are
Devin sessions.

### P-α — Spoke speaks `agentcompanies/v1` *(~1 session)*

The user-facing win: `spoke add` (or `nest roster add`) against a paperclip
team/skill package or git URL.

- `spoke/crates/spoke-core`: `agentcompanies.rs` — parse `COMPANY/TEAM/AGENTS/PROJECT/TASK.md`
  frontmatter + folder convention; build the package graph from `includes[]`.
- Projection rules (the mapping, mechanical):
  - `AGENTS.md` → `.nest/agents/<slug>.md` + `[[bots]]` entry (`name`→name,
    `title`→job, `manager`/`reportsTo`→`boss`, `skills[]`→skills).
  - `SKILL.md` → `skills/<slug>/` (already the same file model — near-passthrough).
  - `PROJECT.md` → `.nest/projects/<slug>.md` (new, see P-β).
  - `TASK.md` → `.nest/tasks/<slug>.md`; `recurring: true` → patrol duty.
  - `TEAM.md` → roster grouping + `boss` wiring down the subtree.
  - `COMPANY.md` → `agent.toml` `[company]` block (name/goals) + `requirements.secrets[]`
    surfaced as a `secrets` manifest checklist.
  - `sources[]` → lockfile provenance entries (same as package fetch records).
- `spoke/crates/spoke-cli`: `spoke add <agentcompanies-ref>` path; preview via
  existing `spoke diff` (the card UI already renders manifest changes).
- Provenance note in `agent.lock`: `source: agentcompanies/v1` + upstream ref.
- Harvest `packages/teams-catalog/catalog/bundled/core-exec-team` (CEO/CTO/QA +
  recurring first-heartbeat task) and `company-defaults/` + `optional/` teams as
  the first packs; smoke-test with the `testing-nest` e2e harness on echo perch.

### P-β — Work units *(~1 session)*

NEST's biggest structural gap: tasks are throwaway strings. Paperclip's `issues`
shape, flattened to files.

- `.nest/tasks/<slug>.md` frontmatter: `title`, `assignee`, `project`,
  `status` (`queued|claimed|working|blocked|done|cancelled`), `parent`,
  `blocked_by[]`, `origin` (manual|patrol|remote|webhook), `created_by`,
  `monitor_at` (re-check wake), `checkout` (run id + timestamp for the claim lock).
- `nest tasks` CLI: list/new/claim/done/block; shell Tasks tab (list + detail,
  work-product links).
- `brood`/`clutch`/`hatchling` accept `task:<slug>` refs; receipt records the task
  slug; claim lock = `checkout` field + atomic rename (mkdir pattern already used
  for wake spools elsewhere in the codebase).
- `tasks/<slug>/artifacts/` — typed work products (`kind: pr|doc|image|data`,
  `review_state`), surfaced in the task view.
- Decomposition (cheap version): `nest plan <task>` prompts the boss bot to emit
  child TASK.md files (paperclip's `issue_plan_decompositions` as files, not a
  service).

### P-γ — Governance *(~1 session)*

- **Approvals**: `ToolPolicy::Ask` (and remote `ask` replies) append to
  `.nest/approvals/<id>.json` (`{type, bot, task, payload, status: pending}`).
  Shell gains an Inbox tab; approve/deny writes `decision` + `note` and wakes the
  blocked run via the P-δ spool. Approvals double as decision-training examples —
  feather can distill an approved/denied pattern into the bot's SKILL context.
- **Budgets**: keep the manifest field, add a real ledger — `~/.nest/usage/` gains
  per-bot `spend` counters (tokens→cents when a perch reports usage; estimate when
  it doesn't). `budget: <cents>` + `budget_window: day|week|month` in `[[bots]]`;
  hatchling refuses a run at hard-stop, warns at 80% (paperclip's `warnPercent` /
  `hardStopEnabled` semantics, file-backed).
- **Secrets manifest**: `.nest/secrets.toml` declares names + which bots may
  reference them (`bindings`); values still `$ENV`/file-backed — never in the
  manifest. Access appended to `events.jsonl` (cheap version of their
  `company_secret_access_events`).

### P-δ — Presence *(~1 session)*

- **Wakeup spool**: `.nest/wakeups/<key>.json` —
  `{source, reason, payload, status: queued, idempotency_key, requested_by,
  coalesced_count}` (paperclip's `agent_wakeup_requests` verbatim). Patrol triggers,
  approvals, remote replies, and monitor wakes all enqueue here; a drain loop in
  `shell` (and `nest drain` for headless) coalesces + dispatches to brood.
  Idempotent by filename — same key merges, bumping `coalesced_count`.
- **Run events persisted**: `roost/<run-id>/events.jsonl` — the typed event stream
  their `heartbeat_run_events` table gives them. Peeps write-through; transcript
  and `spoke why`/feather read from it (provenance upgrade for free).
- **Monitors**: `monitor_at` on tasks (P-β) enqueues a wake at the time — the
  `monitorNextCheckAt`/`monitorWakeRequested`/`monitorAttemptCount` pattern.
- **Inbox roll-up**: shell tab = pending approvals + blocked tasks + failed runs
  (their attention model, minus the table).

### P-ε — Channels *(1–2 sessions)*

- `channels.toml` (spoke-managed): `[[channels]]` = `{name, provider, external_id,
  assigned_bot, publication: automatic|explicit, execution_policy: restricted|agent}` —
  the fields of `chat_channels`, file-native.
- Keep the existing remotes file-spool as the `local` provider; add real
  `telegram`/`discord` bridges as **spoke packages containing a small stdio
  service** (config in channels.toml, secrets via secrets manifest) — modeled on
  paperclip's service logic but thin. Don't port their 200-service stack; a
  channel adapter that can receive + post under policy is the 90% win.
- GitHub provider: reviews/checks/comments → wakeups (their `github` service
  events as `.json` spool entries).

### P-ζ — Org depth *(~1 session)*

- `[[bots]]` gains `reports_to`, `capabilities`, `paused` (+ `pause_reason`) —
  rendered into agent frontmatter; `brood` routing consults `capabilities` text
  and skips paused bots. Org chart view in shell roster (their `reportsTo` tree).
- `[[projects]]` manifest section → `.nest/projects/<slug>.md` (`owner`→bot,
  `tasks/` convention from agentcompanies).
- `nest export` — whole-roster export emitting an `agentcompanies/v1` package
  (COMPANY.md + TEAM.md + AGENTS.md + TASK.md + SKILL.md). This is the inverse of
  P-α and makes NEST a peer in their format ecosystem — interop both ways, no
  code shared.
- `nest announce "<msg>"` — broadcast prepended to each bot's next prompt (their
  `announcements` concept as a file the prompt builder reads once).
- Later: `[[pipeline]]` manifest section (named stage lists with enforced
  transitions) — file first, enforcement in drain.

### What is invoked, not ported

- **Agent adapters**: paperclip's `*-local` adapter set (claude/codex/gemini/grok/
  hermes/kimi/opencode/pi/cursor) + `ADAPTER_SDK` is a superset of the perch model
  (env contracts, skill sync, prompt files, workspace prep/restore). Where a perch
  is thin, a perch can shell out to their adapter entrypoint rather than
  re-implementing invocation semantics. `AUTHORING.md`'s "no-remote-git" workspace
  contract (prepare/restore over ssh) is worth copying *as a contract* for the
  worktree option.
- **Their runtime**: a `paperclip` service can itself be a remote (`remotes/
  paperclip/` inbox) or a perch (`paperclip` perch invoking `pc issue claim`) —
  the two systems can literally interoperate through the file spool.

### Deliberately skipped

Postgres/Drizzle, multi-tenancy (company-as-tenant), auth/session tiers,
telemetry, the plugin entity/job/webhook system (spoke packages cover the
extension need), container sandbox providers (revisit for isolation), the full
heartbeat-contract machinery (execution-workspace leases, ssh sync) — take only
the worktree idea.

## Cross-platform / pull-local guardrails

- No daemons required: every queue is a directory; `nest drain` works headless;
  the shell is optional.
- Paths via `std::path::PathBuf` everywhere; no `/tmp` assumptions, no symlink
  reliance (registry already rejects symlinks), no `chmod`/signal code paths in
  the new surface.
- All new state under `~/.nest/` (or repo `.nest/` where manifest-scoped) with
  the existing atomic-write + backup conventions; schema bumps go through the
  `sessions` v1-style `schema` field.
- agentcompanies parsing is YAML+markdown only — pure Rust crates, no network
  required to project a local package; git URLs use the existing spoke fetch path.

## Effort summary

| Phase | Deliverable | Sessions |
|---|---|---|
| P-α | `agentcompanies/v1` reader + `spoke add` projection + catalog packs | 1 |
| P-β | `.nest/tasks/` model + CLI + Tasks tab + claim lock | 1 |
| P-γ | Approvals spool + real budgets + secrets manifest | 1 |
| P-δ | Wakeup spool + events.jsonl + monitors + inbox | 1 |
| P-ε | `channels.toml` + telegram/discord bridge packs + github events | 1–2 |
| P-ζ | `reports_to`/`capabilities` + projects + `nest export` + announce | 1 |
| — | Pipelines, plan decomposition depth, sandbox providers | later |

Total: ~6–7 sessions for the whole fold, sequenced P-α first (it seeds the
content everything else attaches to). P-β..P-δ are mutually independent; P-ε and
P-ζ can land in any order after P-α.
