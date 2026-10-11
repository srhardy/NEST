# 09 — Genesis vs NEST: integration analysis

*What the Genesis design (a "one assistant" AI-OS spec) adds to NEST, what's
wrong inside NEST today, and whether to start over or integrate. Prepared by
Devin, 2026-10-09, after reading the full `nest/` spine, `spoke/`, `packages/`,
`BIG-PLAN.md`, and the testing-nest skill notes.*

---

## 1. The two products are solving different problems

| | **NEST** | **Genesis (spec)** |
|---|---|---|
| Mental model | A *cast of bots* you watch, chat with, grow | ONE assistant; specialists hidden |
| Execution | Subprocess wraps external agent CLIs (perch) | In-process LangGraph + LLM API calls |
| Routing | keyword substring → fallback ladder → boss | LLM chief-of-staff delegating by tool call |
| Memory | SKILL.md files (write-only learning) | 5-layer: conversation/project/personal/graph/reflection |
| Safety | per-bot tool allow/deny → CLI flags or prompt text | 6-level × 4-mode matrix, interrupts, event-sourced audit |
| Data | flat files (`~/.nest/*.md`, `.toml`) | Postgres + pgvector |
| Audience | "noob-first" bot manager | power-user personal OS |

NEST is a **delivery/distribution substrate** (wrap CLIs, package skills,
schedule runs, stream peeps). Genesis is a **brain** (memory + routing +
policy). They compete only at the "who does the user talk to" layer.

## 2. Verdict up front

**Don't start over. Integrate.** NEST has real, tested shipping code (~5.8k
lines of Rust, unit + integration tests, CI) covering things Genesis hasn't
built: perch abstraction, spoke packaging, feather learning, patrol triggers,
remotes, daemon, Tauri chrome. Genesis has designs NEST fundamentally lacks:
memory, a real routing brain, approvals, audit. Rebuilding NEST's substrate in
Python would be a year of duplicated work.

**The integration path:** NEST's `boss` + `brood` becomes a thin shell around
a Genesis chief-of-staff; perches become execution tools *behind* the
Genesis tool layer; the roster becomes specialists the chief delegates to —
invisible unless the user opens Expert mode. NEST keeps its files/CLI/desktop;
Genesis provides the brain via a small Python sidecar service (or a `genesis`
perch-like crate over HTTP).

## 3. What Genesis adds to NEST (adopt these)

1. **Event-sourced audit.** Today peeps stream over SSE and are *not
   persisted* — receipts keep a 40-line tail only. You cannot reconstruct
   what a bot did. Persist peeps per run (`roost/<id>.peeps.jsonl`) → audit
   log, replay, feather-quality input, and the digital-twin signal, all free.
2. **Approval gate.** `Peep::Ask` exists in the enum but **nothing ever
   emits it** — "ask" silently becomes deny on every perch. Headless runs
   can't pause mid-CLI, so approvals must happen *pre-launch*: classify the
   task's required tools against the bot's `permissions` before `hatch`, and
   surface "needs approval" in the shell instead of denying silently.
3. **LLM routing.** `routing.toml` substring matching is the weakest brain
   in the system: "fix this bug" only routes if the word "bug" is in the
   table. The forage router already exists — add a `brood` mode where a
   cheap/free provider classifies task → bot, with keyword table as
   deterministic fallback.
4. **Memory layers.** ~~NEST's only memory is hand-authored SKILL.md.~~
   **Done (Phase 2):** `~/.nest/memory/` + project `.nest/memory/` —
   `facts.md` learned lines with run-id provenance (post-run forage
   distill, dedup on append), `user_profile.md` hand-editable, injected
   as `## Memory` into every prompt, plus `nest reflect` — a digest +
   prune routine runnable manually, over `/api/reflect`, or as an
   `action: reflect` automation (seeded disabled).
5. **Security levels.** ~~NEST's egress/budget are *prompt text* — the
   harness is politely asked.~~ **Addressed (hardening):** the `Perch`
   trait now declares real capability — `enforces_tools()` (claude flags,
   codex sandbox, opencode `OPENCODE_PERMISSION`, pi `--exclude-tools` →
   true; stubs → false), `enforces_egress()` and `enforces_budget()`
   (no perch has real controls today → always false). `wait()` emits a
   `⚠ … is advisory` peep for each declared-but-unenforceable policy —
   the tape says what was pretend, instead of pretending.
6. **Failure-aware fallback.** `dispatch` climbs the ladder on *any* non-Done
   status. A permission denial or budget kill shouldn't re-run the same task
   on a different bot — classify the failure (spawn | tool-denied | budget |
   crash) and only escalate crash/failed-launch.
7. **Unified search.** ~~Chats + receipts + skills + agents are all files —
   a `nest find <q>` over them (index on roost + skills + sessions) is a
   cheap version of Genesis's universal search.~~ **Done (Phase 3):**
   `nest find`/`/api/find` scans receipts, sessions, agents, skills, and
   memory into one scored stream — plus a persisted `~/.nest/inbox/`
   (approval cards from the launch gate, routine-run notes) rendered as
   cards in the shell.

## 4. Concrete flaws found in NEST today

Ordered by severity:

1. **`Peep::Ask` is dead code** — ~~defined, rendered in `main.rs`, never
   emitted by any perch or the engine. The approval path is entirely
   absent.~~ **Fixed (Phase 1):** the launch-time gate emits `Ask` peeps
   for every unapproved `ask`-permission tool before hatch; `approved_tools`
   on the API or `--yes`/tty-confirm on the CLI promotes them to allow for
   that run only; Phase 3 added the UI cards — denied asks surface as
   inbox items that approve-and-re-run the task promoted.
2. **`max_steps` doesn't work on most perches.** ~~Only `Peep::Step`
   increments the counter; `echo`, `opencode`, and parts of `pi` emit only
   `Output`, so budget enforcement silently never trips.~~ **Fixed (Phase
   0):** `max_seconds` wall-clock cap (default 600) is the universal rail —
   `Peep::TimedOut` → `BudgetExceeded`. Output lines deliberately aren't
   counted as steps: packaged bots ship `max_steps: 15–50` and echo emits
   the whole prompt as Output, which would kill every test drive.
3. **No persisted peeps** — ~~audit/replay impossible (see 3.1).~~ **Fixed
   (Phase 0):** `wait()` writes `<roost>/<id>.peeps.jsonl` (ms-stamped,
   one JSON per event) and receipts carry `peeps_path`.
4. **Sequential ladder only.** ~~`brood.dispatch` tries candidates one at
   a time; there's no fan-out/join across different bots (clutch is
   same-bot ×N).~~ **Partially addressed:** `nest pipe` chains different
   bots (`researcher|writer|reviewer`) — each stage's receipt tail feeds
   the next as `## Pipeline context`, a failed stage stops the chain.
   True fan-out/join across *different* bots remains open (bigger
   architectural piece — dispatch returns receipts, not streams).
5. **Failure-blind fallback** — ~~any non-Done walks the ladder (see 3.6).~~
   **Fixed (Phase 0):** receipts carry `failure_kind`
   (spawn/exit/budget/denied — denial sniffed from the output tail per
   perch); `dispatch` only climbs on spawn/exit and stops on budget/denied.
   Bonus caught by the new tests: `child.kill()` left grandchildren holding
   the pipes — kills now take the whole process group (unix).
6. **Keyword routing is brittle** — ~~substring containment; a paraphrased
   task falls to the boss, and a task matching two routes tries both bots'
   full fallback chains.~~ **Addressed (Phase 1):** `chief.rs` plans over
   the roster via the forage router — chat and `nest brood` go
   chief→routes→boss; routing.toml remains the deterministic fallback
   (`Undecided`) and the Expert `/api/brood` path.
7. **Perch parsing is fragile** — ~~each perch hand-parses one CLI's
   output format; CLI upgrades can silently degrade parsing with no
   detection.~~ **Detection added:** perches that parse structured events
   (`enforces_tools`-style `structured_output()` → claude/codex/pi true)
   now warn on the tape when a whole run produces only plain Output
   lines — "emitted no structured events — output format may have
   changed". Steps/session tracking degrades visibly, not silently.
8. **Session continuity is thin** — ~~transcripts persist but continuation
   "varies by perch" (README's own words): no harness session-resume flags
   are passed, so multi-turn chats restart context each hatch.~~ **Fixed
   (Phase 1):** perches surface the harness's id as `Peep::Session`
   (claude init event, codex `thread.started`) → `Receipt.session` →
   `transcript.harness_session` → the next send passes it back
   (`claude --resume`, `codex exec resume`, `opencode --session`).
9. **Webhook reply path traversal — verify.** `chat:`/`channel:` values now
   pass `name_ok()` before becoming outbox filenames (line ~1398) — the
   finding from the testing skill is patched in this tree. **Locked in
   (Phase 0):** the regression test now covers `../`, absolute paths,
   Windows backslashes, subdirs, and dotted names — all get no outbox write.
10. **`POST /api/expert` writes `agent.toml`** — ~~validated, but worth a
    hostile-input test.~~ **Locked in:** `expert_write_rejects_hostile_input`
    covers traversal/absolute/sibling paths (rejected before write),
    unparseable TOML, parse-but-invalid manifests (`name = ".."` fails
    `Manifest::validate` — never written), and the valid round-trip.
11. **`services.toml` spawns arbitrary commands** with health checks — fine
    for declared services, but it's an execution surface with no egress or
    budget guardrails (same prompt-trust model).
12. **Plaintext tokens** — `~/.nest/remotes/<r>/token` is a raw string on
    disk; consistent with file-first design. **Hardened:** on unix the
    webhook path now warns (loud `⚠` on stderr) when a token file is
    group/world-readable (`chmod 600` hint) — no OS keyring, but the
    permission foot-gun is called out at read time.

## 5. What NEST should *export* to Genesis (keep these)

- **Perch abstraction** — "wrap, don't rebuild" is the right call for
  agent CLIs. Genesis's Developer/Research agents could call perches as
  Execute-level tools instead of rebuilding agent logic.
- **Spoke manifest + lockfile + marked blocks** — a real package manager
  for bots/skills with idempotent, backup-on-write projections. Genesis's
  plugin story is weaker; spoke is the answer.
- **Feather** — distill-run-to-skill is a clean self-improvement loop;
  maps to Genesis's Learning agent.
- **`providers.toml` cost tiers + limit windows** — free-first with
  cooldowns is a nicer model-router policy than static lists.
- **Patrol trigger taxonomy** — `file-change:`, `git-push`, `app:` marker
  files are cheap triggers Genesis's automation layer should copy.
- **Readable file configs** — `agents/*.md`, `settings.md`, `routing.toml`
  are inspectable and git-friendly; keep them as the Expert-mode surface.

## 6. Recommended integration shape

```
User → nest shell (or chat)
        │
        ▼
  chief (NEW — forage/LLM, not routing.toml)
        │ plan → delegate(bot, task)
        ▼
  brood.dispatch — perches unchanged
        │
        ▼
  hatchling → receipt + PERSISTED PEEPS (NEW)
        │
        ▼
  memory distill (NEW — feather generalized)
```

- Phase 1 (low risk): persist peeps, fix `max_steps` counting, classify
  failure kinds in dispatch, `name_ok` test coverage.
- Phase 2: LLM chief — a `chief.md` boss bot whose prompt gets the roster
  + memory; routing.toml becomes the deterministic fallback, not the brain.
- Phase 3: memory files + pre-launch approval gate + universal `nest find`.
- Phase 4: Genesis sidecar (Postgres memory/graph/digital-twin) — only when
  file-based memory proves insufficient; keep it optional.

## 7. Honest risks in the merge

- **Rust ↔ Python boundary.** Genesis is FastAPI/Python; NEST is Rust.
  The pragmatic bridge: keep everything file/SSE-based (NEST already is),
  and let the chief be either (a) a bot that orchestrates via `nest`
  subcommands — ugly but zero-new-code, or (b) an HTTP sidecar the shell
  calls before `brood`. Don't rewrite NEST in Python.
- **Audience conflict.** NEST targets "noob-first with a visible cast";
  Genesis targets "power-user with one assistant." Decide which persona the
  default UI serves — the roster can stay as Expert mode, or the chief can
  be the default with the cast hidden behind it.
- **Files vs DB.** Genesis's memory graph wants Postgres; NEST's
  file-first philosophy is a feature. File-backed memory (~/.nest/memory/)
  covers most of the value; add pgvector only if retrieval proves weak.

---

*Bottom line: NEST is a good delivery vehicle missing a brain and a memory.
Genesis is a brain design missing a delivery vehicle. Keep NEST's engine,
spoke, feather, patrol, desktop — give it the chief-of-staff, persisted
events, approval gate, and memory layers. Starting over would discard the
hardest part (the working substrate) to rebuild the easy part (the specs).*
