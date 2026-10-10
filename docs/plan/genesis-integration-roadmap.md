# Genesis→NEST integration roadmap

*Companion to `docs/09-genesis-integration-analysis.md`. Staged, mergeable
work items — each phase ships green (`cargo build && cargo test` in `nest/`
and `spoke/`) before the next starts. File-first philosophy preserved: no
Postgres, no Python — the Genesis features are rebuilt natively in Rust.*

## What we're NOT changing

- Perches (claude/codex/opencode/pi wrappers) — the "wrap, don't rebuild" bet
  stays.
- Spoke manifest/projection/lockfile — already good.
- File-based state under `~/.nest/` (and `.nest/` project tier) — human-
  readable is a feature; memory goes into files too.
- The roster/bird UX — it becomes the *inspectable* layer under the chief,
  not the thing you manage.

## The end state

```
user ──► chief (LLM via forage, not routing.toml)
           │ plan → delegate(bot, task) · answer directly · ask approval
           ▼
        brood.dispatch → hatchling → receipt + roost/<id>.peeps.jsonl
           │                                    │
           ▼                                    ▼
      memory/ write (post-run distill)    audit + replay + feather input
```

---

## Phase 0 — Hardening sprint (unblocks everything)

*Small, independent, testable fixes to the existing spine.*
**Status: shipped** — all five items landed with tests; nest 87 + spoke
48 tests green. Bonus fix the tests caught: `child.kill()` left
grandchildren holding the output pipes — kills now take the whole
process group and the post-kill drain is bounded (`DRAIN_GRACE`).

| # | Change | Files | Notes |
|---|--------|-------|-------|
| 0.1 | **Persist peeps.** `wait()` writes `roost/<id>.peeps.jsonl` — one JSON per event with ms timestamps — alongside the receipt. | `hatchling.rs`, `roost.rs` | Receipts gain `peeps_path`. Old receipts replay with partial data — that's fine. |
| 0.2 | **Wall-clock cap.** `agent.max_seconds` (default 600) enforced in `wait()` — kills the child when exceeded → `Peep::TimedOut` + `BudgetExceeded`/`failure_kind: budget`. Kills the whole process group (unix) so orphaned grandchildren can't hold the pipes open. | `spec.rs`, `hatchling.rs` | Fixes the "max_steps never trips on output-only perches" bug. Done: Output peeps are NOT counted as steps — a step is a harness work unit, and packaged bots ship `max_steps: 15–50` while echo emits the whole prompt as Output lines (counting them would kill every test drive). The wall clock is the universal rail; `max_steps` semantics documented. |
| 0.3 | **Failure kinds.** `Receipt.failure_kind: Option<enum {spawn, exit, budget, denied}>`; `denied` detected from stderr patterns (`permission`, `denied`, `sandbox`) per perch. | `hatchling.rs`, `perch.rs` | |
| 0.4 | **Failure-aware ladder.** `brood::dispatch` only climbs to the next candidate on `spawn`/`exit` failure. `budget`/`denied` stops immediately — re-running on another bot isn't a retry, it's a different action. | `brood.rs` | |
| 0.5 | **Reply-path test.** Webhook `chat:`/`channel:` → outbox filename — assert `../x` and `/abs` produce no reply file. | `tests/shell_test.rs` | `name_ok()` guard exists; this locks it in. |

**Tests:** `hatchling.rs` mod tests (peeps file written, cap kills run);
`brood.rs` (ladder stops on budget/denied); `shell_test.rs` (traversal).
**Merge gate:** CI green; `nest brood --perch fail "task"` → `BudgetExceeded`-style receipts stop the ladder without spawning the next bot.

---

## Phase 1 — The chief (Genesis's core idea)

*Replace keyword routing with an LLM planner; keep routing.toml as the
deterministic fallback.*

| # | Change | Files | Notes |
|---|--------|-------|-------|
| 1.1 | **`chief.rs`** — new module. `plan(task, roster, memory_ctx, router) -> ChiefPlan` where `ChiefPlan = Answer{text} \| Delegations{Vec<Delegate>} \| AskApproval{Vec<tool>}`. Calls `forage::Router` (cheapest provider) with a structured prompt (roster = name+job, not persona), parses JSON-ish reply, falls back gracefully on bad output. | `chief.rs` (new), `lib.rs` | Delegation is data, not a new subsystem — CrewAI insight: delegating IS just the next dispatch. |
| 1.2 | **`brood.plan_chief()`** — chief delegates → those agents in order, then routing.toml candidates, then boss. `Via::Chief` variant on receipts. | `brood.rs` | Unmatched tasks still fall back to keyword routes → boss — chief is the front, not the only path. |
| 1.3 | **Pre-launch approval.** Bot's `permissions` containing `ask` → shell shows an approval card (the asks + the task) *before* hatch; approve once → those tools promote ask→allow for this run only. `Peep::Ask` gets emitted at gate time, not mid-run (perches can't pause — pre-launch is the honest gate). | `spec.rs`, `shell.rs`, `main.rs` | Removes today's silent ask→deny on every perch. CLI: `--yes` flag or interactive confirm. |
| 1.4 | **Sessions.** `Launch` gains `session_id: Option<String>`; perches that support resume pass it (`claude --resume`, `codex exec resume`, `opencode --session`). Transcript store keeps the harness session id per chat → real multi-turn instead of context-free re-hatches. | `perch.rs`, `hatchling.rs`, `shell.rs` | README's own known gap; echo/fail ignore it. |
| 1.5 | **Chief in the UI.** Chat input defaults to `/api/chief` (was: `/api/brood`); roster stays for direct per-bot work and Expert mode. The boss bird becomes the visible assistant face. | `shell.rs`, `web/index.html` | User sees one assistant; the cast is the trace. |

**Tests:** `chief.rs` (plan parsing incl. malformed output → fallback);
`brood.rs` (chief order beats routes); `shell_test.rs` (approval gate blocks
then releases a run). Deterministic: forage router already has `echo`/`fail`
stubs — add a `scripted` forage for plan tests.
**Merge gate:** `nest brood "…"` uses chief→route→boss ordering with real
providers disabled (echo forage) — proves no-LLM path still works.

---

## Phase 2 — Memory (files, not a database)

*File-first adaptation of Genesis layers 1–3+5.*

| # | Change | Files | Notes |
|---|--------|-------|-------|
| 2.1 | **Memory dirs.** `~/.nest/memory/` (personal) + `.nest/memory/` (project). `facts.md` = one fact/line, `# <run-id>` provenance trailing each line. `user_profile.md` hand-editable. | `home.rs` | Same precedence as agents: project shadows personal. |
| 2.2 | **Post-run extraction.** After `Done` receipts: background distill (forage, cheapest provider) → candidate facts → hash-dedup → append to `facts.md`. Never blocks the run; failures just skip. | `memory.rs` (new), `hatchling.rs`/`shell.rs` | Reuses feather's distill pattern. Async: `thread::spawn` post-receipt. |
| 2.3 | **Prompt injection.** `build_prompt` gains a `## Memory` section: `user_profile.md` + project `facts.md` (capped ~4KB) — every hatch sees what the nest knows. | `perch.rs` | Letta blocks, file edition. |
| 2.4 | **`nest reflect`.** A routine (or manual command) that reads the roost + facts, summarizes activity into `memory/daily/`, prunes stale/duplicate facts (receipt-claimed, not silently deleted — moves to `archive.md`), emits a digest. | `memory.rs`, `main.rs`, `patrol.rs` | Genesis layer 5. Ships as a seeded `[[automation]]` so users get it for free. |
| 2.5 | **Memory UI + `nest memory`.** List/edit facts in the shell (they're files — the UI is a viewer/editor, not a service). | `main.rs`, `shell.rs`, `web/` | |

**Tests:** `memory.rs` (dedup, append, profile injection into `build_prompt`);
reflect over fixture receipts produces digest + archive.
**Merge gate:** run a run stating a preference → fact lands in `facts.md` →
next hatch's prompt contains it → `reflect` prunes a planted duplicate.

---

## Phase 3 — Find everything + approvals polish

| # | Change | Files | Notes |
|---|--------|-------|-------|
| 3.1 | **`nest find <q>`** — grep-level search over agents/skills/roost/memory/sessions, one result stream with type tags. `/api/find` for the shell's command-palette slot. | `find.rs` (new), `main.rs`, `shell.rs` | Cheap now (files), swappable to an index later. |
| 3.2 | **Inbox.** Persisted `~/.nest/inbox/` items — approval requests, digest drops, routine results — the Superhuman triage queue for everything the nest did while you were away. | `shell.rs`, `web/` | Digest from 2.4 lands here. |
| 3.3 | **Approval cards in UI** — pending gate renders as a card (bot, task, tools asked for, approve/deny buttons hitting `/api/approvals`). | `web/` | |

**Tests:** find over fixture home; inbox write/read; approval API round-trip.
**Merge gate:** search returns cross-type hits; a routine run leaves an inbox
item; an ask-permission bot produces an approval card.

---

## Phase 4 (defer) — the Genesis sidecar

Only if file memory proves insufficient: Postgres+pgvector life-graph,
digital-twin distillation, deeper audit queries. The `memory.rs` API
(`facts() / append() / search()`) is the seam — a sidecar implements the
same calls over HTTP, nothing else changes. Decide with data, not ambition.

## Standing rules

- Every phase: unit tests in-module + `tests/` e2e where the shell/daemon
  path is touched; the deterministic perches (`echo`/`fail`) + forage stubs
  keep tests offline.
- `name_ok`-style input validation on every new path boundary — the
  outbox fix is the pattern.
- Docs land with the code (`docs/` updates in the same PR).
- No silent behavior changes: anything advisory (egress/budget) that isn't
  enforceable stays labeled advisory in UI + prompt.
