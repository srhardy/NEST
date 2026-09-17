# About the Spoke project

> **TL;DR — what changes for you**
> - **One file configures every coding agent you use.** Write `agent.toml` once; Spoke projects it into Claude Code, Codex, Gemini CLI, OpenCode and whatever comes next.
> - **Nothing gets clobbered.** Spoke writes inside marked blocks and performs surgical merges — your existing settings, hooks and comments survive. Every change is backed up first.
> - **Uninstall means uninstalled.** Remove a skill package and its files are gone (moved to backups), not "mostly gone, good luck".
> - **You can see the bill.** Every installed skill has a measured context footprint, and overlapping skills get flagged before they quietly cost you tokens forever.
> - **Your machine stays yours.** No telemetry, and Spoke actively switches off the phone-home beacons that other tools leave running.
> - **It is one file.** A single Rust binary on your laptop and your always-on server. No runtime archaeology, no dependency rot, no `node_modules` graveyard.

---

## The problem, in one picture

```
        WITHOUT SPOKE                              WITH SPOKE
  ┌──────────────┐                          ┌──────────────────┐
  │  skills dirs │──┐                       │    agent.toml    │  (one source of truth)
  │  hooks       │  │                       └────────┬─────────┘
  │  MCP config  │  ├──▶ 5 different                │
  │  instructions│  │    config formats,    ┌───────▼────────┐
  │  permissions │  │    5 mental models    │      Spoke     │
  │  package mgr │  │                       └───────┬────────┘
  └──────────────┘  │                       ┌───────┴────────┐
        per tool ───┘                       ▼        ▼       ▼
                                       Claude    Codex   Gemini/OpenCode
                                       Code                      (planned)
```

Every harness invented its own answer to the same six questions — instructions, skills, commands, hooks, MCP servers, permissions — and none of the answers are compatible. The result is not just busywork: it is silent drift, duplicated context, and configuration that no human can audit.

---

## Why the current generation of agent tooling fails

These are **categories** of tooling, not names — and the flaws are inherited, not incidental:

| Category | Inherited flaw | What it costs you |
|---|---|---|
| Skill / prompt packs | Installed by copying files; no version, dependency or removal story | Duplicates pile up, context balloons, uninstall never happens |
| Token-shaver add-ons | Optimise before measuring; quality claims are unevidenced | You pay in quality for savings you never verified |
| Per-harness adapters | Single-vendor by construction; every tool re-implements its own adapter | "Support X?" is the most-requested issue in the whole ecosystem |
| Memory / context layers | Lock you to one harness's hook model | Handoff between vendors is the one thing they all promise and none deliver |
| Web & document fetchers | Cloud dependency, hostile install, or licence traps | The most useful layer is the one you cannot run locally |
| Curated registries | Static lists that rot; verification is a human reading a diff | Dead entries, promotional spam, no integrity signal |

Common root cause: **each tool is a patch on one harness, so every user ends up maintaining a portfolio of patches.** Spoke is not another patch. It is the layer the patches should have been written against.

---

## Design decisions, and why

| # | Decision | Justification |
|---|---|---|
| 1 | **One manifest, many projections** | Configuration drift is the failure mode of multi-tool setups. A projection model makes "what does my agent know?" answerable in one file, reviewable in a pull request. |
| 2 | **Never clobber: marked blocks + surgical merges** | User config is the most expensive artefact in the room. Any tool that eats a settings file is uninstalled immediately, so non-destructive behaviour is the product's licence to operate. |
| 3 | **Append-only and marker-scoped edits** | Hooks and arrays are where users already have their own entries. Appending means coexistence; a marker means removal touches only Spoke's own additions. |
| 4 | **Backups between plan and apply; deletions are moves** | Reversibility converts a scary tool into a boring one. `--dry-run` shows the diff; backups make "undo" a file operation, not a hope. |
| 5 | **Rust core, single binary, thin JS shims only where a harness demands them** | Durability is a feature. One file to install on a laptop and a server, no runtime version roulette, fast startup, no supply-chain surface from package trees. |
| 6 | **The manifest is a projection model, not a lowest common denominator** | Harnesses genuinely disagree (hooks, permissions, MCP shapes). Spoke's job is to project what each can express and **warn** when something cannot be represented — never to silently drop it. |
| 7 | **A package is a directory, a lockfile, and a content hash** | Reproducibility without a server: a git tap is the registry, the lockfile is the contract, `sha256` is the tripwire. Signed indexes come when hosting is justified, not before. |
| 8 | **Measure before optimising; label every estimate** | The category is saturated with unproven claims. Spoke reports measured footprints, marks heuristics as estimates, and refuses to sell a saving it has not demonstrated. |
| 9 | **Hooks observe; they do not police** | Enforcement belongs in the host harness, where it cannot be bypassed by a wrapper. Spoke uses hooks for accounting and continuity, and is explicit that its scan is advisory, not a sandbox. |
| 10 | **Local-first, telemetry-free, beacons off** | Trust is asymmetric: you can gain it slowly and lose it once. Spoke ships no telemetry, and sets the standard do-not-track variables so third-party beacons inherited into your environment stay quiet. |
| 11 | **One event log behind every feature** | Costs, audit history, and future quality scores all read the same append-only stream, so features cannot disagree about what happened. |
| 12 | **Everything reversible, opt-in, and inspectable** | `doctor` shows drift and capability gaps; `why` shows provenance and re-verifies content hashes; nothing changes behaviour without a visible command. |

---

## How it works, in practice

```
  plan ──▶ preview ──▶ backup ──▶ apply ──▶ verify
   │         │           │          │         │
   │         │           │          │         └── doctor / diff: zero drift?
   │         │           │          └── writes only what changed, idempotent
   │         │           └── ~/.spoke/backups/<timestamp>/…
   │         └── spoken plainly: "would update X — why"
   └── agent.toml → per-harness projection, capability gaps warned
```

Day one, three commands:

```sh
spoke init                 # detect harnesses, write agent.toml, prepare the store
spoke diff                 # see exactly what would change, in plain language
spoke sync                 # apply it (backups taken first)
```

Then, when you want skills:

```sh
spoke add <package>        # fetch, scan, measure, install into every harness
spoke dedupe               # flag near-duplicate skills you are paying for twice
spoke budget --receipts    # what your context costs, per package
spoke remove <package>     # gone: files moved to backups, lockfile updated
```

---

## What is shipped today, and what is next

| Area | Status |
|---|---|
| Sync: instructions, MCP servers, telemetry-off env | **Shipped** — Claude Code, Codex |
| Package manager: add / remove / list / why / dedupe, lockfile, integrity | **Shipped** |
| Budget: event log, harness hooks, context footprint, overlap savings | **Shipped** (measurement only) |
| Gemini CLI + OpenCode adapters, VPS serving mode, signed auto-update | Planned next |
| MCP server surface, document/web fetch spoke, memory + graph + policy spokes | Planned |
| Context compression, published quality benchmarks, signed registry | Planned |

---

## Security, privacy, reversibility

- **What Spoke touches:** a documented, finite set of files under your home and project directories — nothing else.
- **What it never does:** phone home, read credentials, or execute package code. Packages are scanned and hashed, not run.
- **What it admits:** scanning is heuristic, integrity is a content hash rather than a signature, and a curious symlink in a hostile package is a known limitation. See `docs/SECURITY.md` for the full threat model and the exact caveats.

---

## The point

A wheel is not strong because one spoke is strong. It is strong because the spokes are true, evenly tensioned, and attached to a hub that does not move.

Every coding agent you adopt is a new spoke. Today, each one arrives with its own hub, its own tools, and its own way of bending your configuration out of shape — so you end up with a pile of wheels and no cart.

Spoke is the hub. One manifest holds the truth; every harness is a spoke that can be added or removed without rebuilding the vehicle. The promise is not that it will do something new and clever for you. It is that it will make the tools you already chose **add up** — measurably, reversibly, and without asking you to learn another configuration language.

That is the whole design: a faster, truer wheel — and every spoke you bring, honestly attached.
