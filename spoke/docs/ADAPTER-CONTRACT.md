# ADAPTER-CONTRACT.md — how a harness plugs into Spoke

An adapter is a **pure projection**: `(manifest, home, project)` → `Plan` (a list of operations). It performs no I/O of its own, holds no state, and never touches the network. Everything a user's machine sees is produced by the operation engine, which is where backups, idempotency and change reporting live.

---

## 1. The trait

```rust
pub trait Harness {
    fn id(&self) -> &'static str;                       // "claude-code" (stable, used in agent.toml)
    fn display(&self) -> &'static str;                  // "Claude Code"   (human output)
    fn capabilities(&self) -> Capabilities;             // what this harness can express
    fn root(&self, home: &Path) -> PathBuf;             // user-level config root (detection + backups)
    fn detect(&self, home: &Path) -> bool { self.root(home).exists() }
    fn skills_root(&self, home: &Path) -> PathBuf { self.root(home).join("skills") }
    fn plan(&self, ctx: &Ctx<'_>) -> Plan;
}
```

`Ctx` carries `home`, `project` and the parsed `Manifest`, plus `instruction_lines()` — the rendered instruction text an adapter should bridge.

---

## 2. Capabilities

```rust
Capabilities { instructions, skills, commands, hooks, mcp, permissions, env }
```

- `true` means the harness has a real, documented mechanism for that surface **and** the adapter projects it.
- `false` means the harness cannot express it (or the adapter does not yet) — and `plan()` **must** emit a warning explaining the gap.
- Capabilities are surfaced by `spoke doctor`; they are a promise to the user, not decoration. Never set `true` to look complete.

---

## 3. The operation catalogue

Adapters do not write files; they return operations. Every operation is idempotent and reversible.

| Operation | Semantics | Backup on apply |
|---|---|---|
| `WriteBlock { path, id, contents }` | Splices `contents` between `<!-- spoke:begin id -->` / `<!-- spoke:end id -->`; content outside the markers is never touched. Creates the file if absent. | Yes (existing file) |
| `MergeJson { path, pointer, value }` | Sets `value` at `pointer`, creating intermediate objects. Unknown keys elsewhere survive. | Yes |
| `MergeJsonArrayAppend { path, pointer, value }` | Appends array elements that are **not already present** (deep equality). Never reorders or replaces existing entries; creates the array if absent. | Yes |
| `RemoveJsonArrayItems { path, pointer, marker }` | Removes array elements whose serialized form contains `marker`. If the path is missing, nothing is created and the file is left alone. | Yes |
| `MergeToml { path, pointer, table_toml }` | Replaces/creates the table at `pointer`, preserving other tables and comments. *(Caveat: a trailing end-of-file comment can be relocated by the TOML engine; content is preserved.)* | Yes |
| `WriteFiles { root, name, files }` | Writes `root/name/<relative>` for each file — used to install skill directories. | Yes (per file) |
| `DeleteDir { path }` | Moves the directory into the backup store (a rename, not a copy-then-delete). | The tree itself |

Change reporting is uniform: `apply`/`preview` return `Change { path, kind, why, changed }`, and the CLI groups them per file so a user sees "3 files changed", not "9 operations".

---

## 4. The contract (rules an adapter must satisfy)

1. **Never clobber.** Use the operations above. Whole-file overwrites of user-owned files are not permitted; managed regions are marked, merges are surgical, additions are appended.
2. **Report, don't drop.** If the harness cannot express something the manifest asks for, warn with a specific message (`plan.warn(...)`). Silent loss is the worst possible behaviour.
3. **Be idempotent.** A second `apply` of the same plan must produce zero changes. No timestamps, no random ordering, no absolute paths that vary between runs.
4. **Order deterministically.** Emit operations in a stable order (manifest order for user data, then fixed built-ins) so diffs are readable.
5. **No I/O, no state, no network.** Adapters read nothing themselves; the engine owns the filesystem so backups and previews cannot be bypassed.
6. **Paths come from `Ctx`.** Never read `$HOME` or the current directory directly — this is what makes every adapter testable and what makes `--home`/`--project` honest.
7. **Prefer the harness's native truth.** Write into the file the harness actually reads (`AGENTS.md`, `CLAUDE.md`, `.mcp.json`, `config.toml`); do not invent a Spoke file and hope the harness notices.
8. **One skill per directory.** Skills are installed as `<skills_root>/<name>/<files>` so removal is exact.

---

## 5. Required tests for any adapter

| Test | What it proves |
|---|---|
| Golden plan | Given a fixed manifest, the plan's file contents match expectations byte-for-byte |
| Preservation | Pre-existing user content (instructions, unrelated keys, comments, other hooks) survives untouched |
| Idempotency | `apply` twice → the second run reports zero changed files |
| Capability gaps | Unsupported surfaces appear as warnings, not omissions |
| Detection | `detect()` is true only when the harness's config root exists (use a temp home) |
| Removal | Removing what was installed restores the prior state and leaves user content intact |

The existing suite (`crates/spoke-adapters/tests/`) is the template: `plan_test.rs` for projections, `ops_test.rs` for operation semantics.

---

## 6. Skeleton

```rust
use std::path::{Path, PathBuf};
use spoke_core::ir::{Capabilities, Op, Plan};
use crate::{Ctx, Harness, TELEMETRY_ENV};

pub struct NewHarness;

impl Harness for NewHarness {
    fn id(&self) -> &'static str { "new-harness" }
    fn display(&self) -> &'static str { "New Harness" }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            instructions: true,
            skills: true,
            commands: false,   // honest: not supported / not implemented
            hooks: false,
            mcp: true,
            permissions: false,
            env: false,
        }
    }

    fn root(&self, home: &Path) -> PathBuf { home.join(".new-harness") }

    fn plan(&self, ctx: &Ctx<'_>) -> Plan {
        let mut plan = Plan::default();

        let instructions = ctx.instruction_lines();
        if instructions.trim().is_empty() {
            plan.warn("no instructions declared in agent.toml — skipping the instruction file");
        } else {
            plan.push(Op::WriteBlock {
                path: ctx.project.join("AGENTS.md"),
                id: "instructions".to_string(),
                contents: instructions,
                why: "project instructions".to_string(),
            });
        }

        for server in &ctx.manifest.mcp {
            // …project into this harness's native shape…
            let _ = server;
        }

        plan.warn("hooks are not projected for this harness yet");
        plan
    }
}
```

Register it in `registry()` (`crates/spoke-adapters/src/lib.rs`) — that single list is what `init`, `sync`, `doctor`, `add` and `remove` all consult, so a new harness immediately works with every existing command.

---

## 7. What the contract deliberately excludes

| Non-goal | Why |
|---|---|
| Adapters performing migrations or repairs | Repairs are the engine's job, with backups and change reporting |
| Adapters enforcing permissions or policy | Enforcement belongs in the host harness (Spoke observes; it does not police) |
| Adapters downloading or executing anything | Fetching is the registry's job; execution is never Spoke's |
| Per-adapter CLI flags | Behaviour is driven by `agent.toml` only, so one command works everywhere |
