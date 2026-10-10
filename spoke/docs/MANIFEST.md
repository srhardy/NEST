# MANIFEST.md — every file format Spoke reads or writes

Four formats, one contract each. If a field is not listed here, Spoke ignores it (and says so).

---

## 1. `agent.toml` — the project truth

Lives in the project directory (the directory you pass as `--project`, default: current directory).

```toml
version = 1

# Harnesses to project into. Unknown names are reported and skipped.
harnesses = ["claude-code", "codex"]

# Reserved: declared skill packages. Installation is done by `spoke add`
# (declaring here is warned about, not silently ignored).
skills = []

# Extra instruction lines, bridged into every harness.
# Long-form project instructions belong in AGENTS.md, which Spoke edits only
# inside its marked block.
[instructions]
extra = ["Verify results independently rather than assuming success."]

# MCP servers, registered in each harness's native format.
# Exactly one of `command` (stdio) or `url` (remote) is required.
[[mcp]]
name = "spoke-fetch"
command = "spoke"
args = ["mcp", "fetch"]

# Token/context policy. Stored today; enforcement lands with the budget work.
[budget]
preset = "balanced"   # concise | balanced | full

# Bots — the roster. Nest renders each into .nest/agents/<name>.md
# (YAML frontmatter + persona body, fully Spoke-owned).
[[bots]]
name = "eggbot"
title = "Dr. Eggbot"
job = "Walks you through building a bot, then writes it"
voice = ["You are a friendly bot-builder who asks one question at a time."]
anti_jobs = ["Do not write application code"]
skills = ["bot-authoring"]        # skill names — warns if undeclared
mcp = ["spoke-fetch"]            # [[mcp]] names — warns if undeclared
model = "balanced"               # concise | balanced | full (hint)
memory = ["Templates live in the catalog"]
mode = "primary"                 # primary | subagent — subagents spool on demand
boss = true                      # at most one — the orchestrator/default receiver
max_steps = 30                   # per-run step budget
max_seconds = 900                # per-run wall-clock budget (engine default: 600)
permissions = { bash = "ask", edit = "ask", read = "allow" }   # allow | ask | deny per tool

# Routing — the Boss's dispatch table: patterns -> bot + escalation ladder.
# Nest renders it to .nest/routing.toml (fully Spoke-owned).
[[routing]]
patterns = ["debug", "fix bug", "stack trace", "crash"]
bot = "debugger"                 # [[bots]] name — warns if undeclared
fallback = ["llm-council"]       # escalation chain — each warns if undeclared

# Services — declared daemons (memory, crawlers…). Nest renders them to
# .nest/services.toml so the app can start and health-check them.
[[services]]
name = "openmemory"
port = 8765
health = "/api/v1/stats/"
start = "docker compose -f ~/mem0/openmemory up -d"

# Automations — trigger-driven runs. Exactly one of `schedule` or `event`.
# Nest renders them into .nest/automations.toml (fully Spoke-owned).
[[automations]]
name = "morning-triage"
schedule = "0 9 * * *"            # 5-field cron
prompt = "Summarize yesterday's changes"
bot = "eggbot"                    # optional [[bots]] name — warns if undeclared
enabled = true
```

| Field | Type | Required | Default | What `spoke sync` does with it |
|---|---|---|---|---|
| `version` | integer | yes | — | Validated; must be `1` |
| `harnesses` | string[] | no | `["nest", "claude-code", "codex"]` | Each entry is resolved to an adapter; enabled-but-not-detected harnesses are skipped with a note |
| `skills` | table[] | no | `[]` | **Reserved** — a non-empty list produces a warning; use `spoke add` |
| `instructions.extra` | string[] | no | `[]` | Written into the marked block of each harness's instruction file (Codex: `AGENTS.md`; Claude Code: `CLAUDE.md`, which imports `AGENTS.md`) |
| `mcp[]` | table[] | no | `[]` | Merged into each harness's native MCP config |
| `mcp[].name` | string | yes | — | Server key in the generated config |
| `mcp[].command` / `args` | string / string[] | one of | — | stdio server |
| `mcp[].url` | string | one of | — | remote server |
| `budget.preset` | string | no | `"balanced"` | Shown by `doctor`/`budget` |
| `bots[]` | table[] | no | `[]` | Nest: rendered to `.nest/agents/<name>.md` |
| `bots[].name` | string | yes | — | lowercase-kebab (a-z, 0-9, `-`, `_`); becomes the filename |
| `bots[].job` | string | yes | — | The one job this bot is for; empty is an error |
| `bots[].title` / `voice` / `anti_jobs` / `memory` | string / string[] | no | — | Display name, persona lines, not-my-job list, seeded notes |
| `bots[].skills` / `mcp` | string[] | no | `[]` | Names resolved against `skills[]`/`[[mcp]]`; dangling refs warn |
| `bots[].model` | string | no | — | Model tier hint (`concise`/`balanced`/`full`) |
| `automations[]` | table[] | no | `[]` | Nest: rendered to `.nest/automations.toml` |
| `automations[].name` | string | yes | — | Unique across automations |
| `automations[].schedule` / `event` | string | one of | — | 5-field cron, or a free-form event string |
| `automations[].prompt` | string | yes-ish | `""` | What the run does |
| `automations[].bot` | string | no | — | `[[bots]]` name to run as; dangling ref warns |
| `automations[].enabled` | bool | no | `true` | Rendered into the file; the runner honours it |
| `bots[].mode` | `primary`/`subagent` | no | — | Lifecycle: subagents spool as delegated subtasks and auto-clean |
| `bots[].boss` | bool | no | `false` | The orchestrator — default receiver for un-routed intents; max one |
| `bots[].max_steps` | int | no | — | Per-run step budget |
| `bots[].max_seconds` | int | no | engine default 600 | Per-run wall-clock budget — works on harnesses that never report steps |
| `bots[].permissions` | map tool→`allow`/`ask`/`deny` | no | `{}` | Per-tool policy rendered into the agent frontmatter |
| `routing[]` | table[] | no | `[]` | Nest: rendered to `.nest/routing.toml` |
| `routing[].patterns` | string[] | yes | — | Keywords/patterns that trigger the route |
| `routing[].bot` | string | yes | — | Target `[[bots]]` name; dangling ref warns |
| `routing[].fallback` | string[] | no | `[]` | Escalation ladder tried in order on failure |
| `services[]` | table[] | no | `[]` | Nest: rendered to `.nest/services.toml` |
| `services[].name` | string | yes | — | Unique across services |
| `services[].port` / `health` / `start` | int / string / string | no | — | Listen port, health-check path, start command |

**Validation rules**

| Rule | Error |
|---|---|
| `mcp[].command` XOR `mcp[].url` | `mcp server 'x' must set exactly one of command (stdio) or url (remote)` |
| `bots[].name` lowercase-kebab | `bot name 'x' must be lowercase-kebab … it becomes a filename` |
| `bots[].job` non-empty | `bot 'x' must declare a job — one sentence, one job` |
| `bots[].name` unique | `duplicate bot name 'x'` |
| `automations[].schedule` XOR `event` | `automation 'x' must set exactly one of schedule (cron) or event` |
| `automations[].schedule` 5 fields | `automation 'x' schedule 'y' is not a 5-field cron expression` |
| `automations[].name` unique | `duplicate automation name 'x'` |
| at most one `boss = true` | `at most one bot may be marked 'boss = true'` |
| `routing[].patterns` non-empty | `route for bot 'x' has no patterns — add at least one` |
| `routing[].bot` / `fallback` names | dangling refs **warn**, never drop |
| `services[].name` non-empty + unique | `service has an empty name` / `duplicate service name 'x'` |
| Unknown harness id | reported during sync, skipped |
| Unknown top-level keys | ignored (forward compatible) |

**Idempotency guarantee:** running `spoke sync` twice changes nothing the second time. Instruction files are edited only between `<!-- spoke:begin instructions -->` and `<!-- spoke:end instructions -->`; everything outside those markers is untouched.

---

## 2. `package.toml` — a skill package

Point `spoke add` at a directory containing this file, or at an index entry that resolves to one.

```toml
name = "answer-first"
version = "0.1.0"
license = "MIT"
description = "Answer-first output rules for coding agents."
tier = "curated"                  # system | curated | experimental (informational)

[source]
path = "./src"                    # or: git = "https://github.com/…"; optional rev = "v1.2.0"

[[skills]]
name = "answer-first"
path = "."                        # relative to [source]; defaults to "."

[[agents]]                        # bot definitions — directories or single .md files
name = "eggbot"
path = "agents/eggbot"

[requires]                        # reserved; non-empty produces a note at install time
mcp = []
```

| Field | Required | Notes |
|---|---|---|
| `name`, `version` | yes | Non-empty. Together they form the install path `~/.spoke/packages/<name>/<version>/` |
| `license` | no | Recorded in the lockfile and shown by `list`/`why` |
| `description` | no | Shown at install time |
| `tier` | no | Informational; mirrors the tiering convention used by existing catalogs |
| `source.path` | one of | Relative paths resolve against the manifest's own directory. Copied into the store at install time (source edits never mutate an installed copy) |
| `source.git` (+`rev`) | one of | `git clone --depth 1 [--branch rev]` |
| `skills[]` / `agents[]` | yes (≥1 combined) | Each becomes one directory per harness (skills → skills root, agents → agents root) |
| `skills[].path` / `agents[].path` | no | Directory relative to the fetched source root; an agent path may also be a single file |
| `requires.mcp` | no | **Reserved** — printed as "not wired yet" |

**Validation errors:** empty name/version · no source · both `path` and `git` · no skills and no agents · empty skill/agent name · skill/agent path missing in source.

**Install semantics**

```
spoke add <path|name|url>
  ├─ spec      → https?:// or file:// URLs are fetched (curl) to ~/.spoke/tmp/;
  │              a bare .md file wraps as a one-agent package (name from
  │              frontmatter, else filename)
  ├─ resolve   → manifest validated
  ├─ fetch     → ~/.spoke/packages/<name>/<version>/   (copy or git clone)
  ├─ collect   → text files only; binary/unreadable files are skipped
  ├─ scan      → warnings: scripts, network references, declared hooks
  ├─ measure   → token estimate = bytes/4  (labelled as an estimate)
  ├─ hash      → sha256 over sorted (path, content) pairs
  ├─ plan      → per harness: <skills_root>/<skill>/<files>
  ├─ apply     → backups first, then writes (idempotent)
  └─ lock      → agent.lock updated
```

---

## 3. `index.toml` — a tap-style registry

Location: `~/.spoke/registry/index.toml`. A registry is any git repo you clone there.

```toml
[[package]]
name = "answer-first"
manifest = "packages/answer-first/package.toml"   # relative to the index directory
```

`spoke add <name>` resolves through this file; `spoke add <path>` bypasses it entirely. This keeps the first version serverless: the index is reviewable, diffable, and hosted anywhere git is.

---

## 4. `agent.lock` — resolved state (JSON, commit me)

```json
{
  "version": 1,
  "packages": [
    {
      "name": "answer-first",
      "version": "0.1.0",
      "source": "path:/srv/pkgs/answer-first/src",
      "license": "MIT",
      "integrity": "sha256:512305af…",
      "tokens": 75,
      "skills": ["answer-first"],
      "agents": ["eggbot"],
      "installed_paths": [
        "/Users/you/.claude/skills/answer-first",
        "/Users/you/.codex/skills/answer-first"
      ]
    }
  ]
}
```

| Field | Meaning |
|---|---|
| `integrity` | `sha256` over the package's skill files, order-independent. `spoke why <name>` re-reads the cached package and reports `ok` or `DRIFT` |
| `tokens` | Estimated context cost (bytes/4) |
| `installed_paths` | Exact directories Spoke created — this is what `spoke remove` deletes (moves to backups) |
| `skills` | Skill names, used for overlap detection and cleanup |
| `agents` | Agent (bot) names installed by the package, shown by `list`/`why` and removed with it |

**Uninstall contract:** removing a package deletes only paths recorded here (plus the same paths re-derived from currently enabled harnesses), moves them into `~/.spoke/backups/`, and drops the lock entry. Nothing else is touched.

---

## Directory layout

```
~/.spoke/
├── backups/<unix-seconds>/<flattened-path>   # every file Spoke was about to change, plus deleted trees
├── events.jsonl                              # append-only: {"ts","kind","detail","cwd"}
├── packages/<name>/<version>/                # frozen copy of each fetched package
└── registry/index.toml                       # optional tap-style index

<project>/
├── agent.toml        # the truth (commit me)
├── agent.lock        # resolved state (commit me)
├── CLAUDE.md         # marked block only
├── AGENTS.md         # marked block only
├── .mcp.json         # merged keys only
└── .claude/settings.json   # merged keys only (env, hooks)

.nest/                # Nest's surface (project-scoped where shown)
├── agents/<name>.md  # generated from [[bots]] — Spoke-owned, edit agent.toml
└── automations.toml  # generated from [[automations]] — Spoke-owned
```

---

## Telemetry-off variables

Set by `spoke init`/`sync` for harnesses that support environment injection (currently Claude Code's project settings), so third-party beacons inherited into your environment stay quiet:

| Variable | Value |
|---|---|
| `DO_NOT_TRACK` | `1` |
| `SUPERPOWERS_DISABLE_TELEMETRY` | `1` |
| `HEADROOM_BEACON` | `off` |
| `SCARF_NO_ANALYTICS` | `1` |
| `RTK_TELEMETRY_DISABLED` | `1` |

Other harnesses have no env projection; Spoke reports that as a capability gap rather than pretending otherwise (see `HARNESS-MATRIX.md`).

---

## Reserved fields (documented so they are not mistaken for behaviour)

| Field | Status |
|---|---|
| `agent.toml` → `skills` | Warned; install via `spoke add` |
| `agent.toml` → `budget.preset` | Stored and displayed; not yet enforced |
| `package.toml` → `requires.mcp` | Warned; Spoke's MCP surface is not built yet |
| `package.toml` → `tier` | Informational only |
