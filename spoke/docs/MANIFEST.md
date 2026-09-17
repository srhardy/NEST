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
```

| Field | Type | Required | Default | What `spoke sync` does with it |
|---|---|---|---|---|
| `version` | integer | yes | — | Validated; must be `1` |
| `harnesses` | string[] | no | `["claude-code", "codex"]` | Each entry is resolved to an adapter; enabled-but-not-detected harnesses are skipped with a note |
| `skills` | table[] | no | `[]` | **Reserved** — a non-empty list produces a warning; use `spoke add` |
| `instructions.extra` | string[] | no | `[]` | Written into the marked block of each harness's instruction file (Codex: `AGENTS.md`; Claude Code: `CLAUDE.md`, which imports `AGENTS.md`) |
| `mcp[]` | table[] | no | `[]` | Merged into each harness's native MCP config |
| `mcp[].name` | string | yes | — | Server key in the generated config |
| `mcp[].command` / `args` | string / string[] | one of | — | stdio server |
| `mcp[].url` | string | one of | — | remote server |
| `budget.preset` | string | no | `"balanced"` | Shown by `doctor`/`budget` |

**Validation rules**

| Rule | Error |
|---|---|
| `mcp[].command` XOR `mcp[].url` | `mcp server 'x' must set exactly one of command (stdio) or url (remote)` |
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
| `skills[]` | yes (≥1) | Each skill becomes one directory per harness |
| `skills[].path` | no | Skill directory relative to the fetched source root |
| `requires.mcp` | no | **Reserved** — printed as "not wired yet" |

**Validation errors:** empty name/version · no source · both `path` and `git` · no skills · empty skill name · skill path missing in source.

**Install semantics**

```
spoke add <path|name>
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
