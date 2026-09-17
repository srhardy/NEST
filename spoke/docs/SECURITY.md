# SECURITY.md — what Spoke touches, what it refuses to do, and where the sharp edges are

Spoke edits the configuration of tools that hold real authority on your machine, so its security posture is part of the product, not an appendix. This document states the model plainly — including the parts that are not solved yet.

---

## 1. Guarantees

| Guarantee | Mechanism |
|---|---|
| No telemetry, ever | Spoke makes no network calls except `git clone` when a package's source is a git URL. Nothing is reported anywhere. |
| No credentials read out | Spoke reads the configuration files it edits and never prints their contents — change reports contain paths and reasons, not values. |
| No code execution | Installing a package copies and scans files. Spoke never runs package code, hooks or scripts it installs. |
| No destructive writes | Every file is backed up before modification; directory removals are **moves** into the backup store, not deletions. |
| No hidden changes | `spoke diff` / `--dry-run` show the exact plan; `apply` reports what changed per file; `doctor` reports drift. |
| Fully reversible | Backups under `~/.spoke/backups/<timestamp>/`, `spoke hooks --uninstall`, `spoke remove <package>`. |

---

## 2. What Spoke touches

| Path | Access | Notes |
|---|---|---|
| `<project>/agent.toml`, `agent.lock` | read/write | Your configuration truth; commit both |
| `<project>/AGENTS.md`, `CLAUDE.md` | read/write | Only inside the marked block; everything outside is preserved |
| `<project>/.mcp.json` | read/merge | Only the named server keys |
| `<project>/.claude/settings.json` | read/merge | Only the specific `env` keys and Spoke's own hook entries (appended, marker-scoped) |
| `<project>/.codex/config.toml` | read/merge | Only `[mcp_servers.<name>]` tables |
| `<home>/.claude/skills/<skill>/`, `<home>/.codex/skills/<skill>/` | write/remove | Installed and removed as whole directories |
| `~/.spoke/{backups,packages,registry,events.jsonl}` | read/write | Spoke's own store |

Everything else is untouched. The list above is exhaustive: if it is not in this table, Spoke does not write it.

---

## 3. Threat model

| Threat | Current mitigation | Residual risk |
|---|---|---|
| **Malicious package** (script that runs on the host, or exfiltrating content) | Static scan warns about scripts, network references and declared hooks **before** install; every install prints these warnings | Scanning is heuristic, not a sandbox. Untrusted packages should be read, not installed |
| **Tampered package after install** | `sha256` integrity over sorted skill files, recorded in the lockfile; `spoke why <name>` re-verifies and reports `DRIFT` | Integrity is a consistency check, **not a signature** — it proves a package changed, not who authored it |
| **Config clobbering** (Spoke itself destroying your setup) | Marked blocks, surgical merges, append-only arrays, marker-scoped removal, backups before every write | TOML trailing-comment relocation (cosmetic) |
| **Path traversal via package content** | Installed paths are constructed as `<skills_root>/<skill>/<relative-from-walk>`, never from package-supplied names | Directory **symlinks** inside a package are followed when collecting files. A hostile package could therefore read files outside its tree into the install. Mitigation: scan warnings + install only packages you trust (documented, not yet fixed) |
| **Secrets in backups** | Backups inherit the permissions of the originals | `~/.spoke/backups/` may contain copies of files that hold API keys (e.g. `env` blocks). There is no automatic retention or encryption — delete the directory when done |
| **Log leakage** | `events.jsonl` records timestamp, event kind, an optional detail string and the working directory | It records **no prompts, no file contents, no arguments**. Review it if you consider paths sensitive |
| **Supply-chain via Spoke's own dependencies** | Small dependency set (serde, toml, clap, sha2), single static binary | Standard Rust ecosystem risk |
| **A hook that fails and blocks your session** | Installed hooks only invoke `spoke event …`, which appends one line and exits | If the binary moves, the hook errors; remove with `spoke hooks --uninstall` and reinstall after moving it |

---

## 4. Package trust model (state it honestly)

```
  author ──▶ git repository ──▶ fetch (clone/copy) ──▶ scan (warn) ──▶ hash (sha256) ──▶ install
                 │                                        │                            │
             provenance you                       heuristics you                drift detection
             chose to fetch                       should read                   you can verify
```

- **What exists:** provenance (you picked the source), a content hash, drift detection, and advisory scanning.
- **What does not exist yet:** signatures, a trusted-key model, allow/deny policy, and sandboxing. Until then, treat package installation exactly like installing software from a repository you have read.
- **What this means in practice:** `spoke add --dry-run` first; read the scan output; prefer packages whose `SKILL.md` you have actually read.

---

## 5. Reversibility runbook

| Situation | Action |
|---|---|
| A sync did something you dislike | Restore the file from `~/.spoke/backups/<timestamp>/` |
| Hooks misbehaving | `spoke hooks --uninstall` (removes only Spoke's entries) |
| A package should go away | `spoke remove <name>` — directories moved to backups, lockfile updated |
| You want to see before you commit | `spoke diff`, `spoke add --dry-run` |
| You suspect tampering | `spoke why <name>` re-verifies the recorded integrity of the cached package |
| You want Spoke gone entirely | `spoke hooks --uninstall`, `spoke remove` each package, delete `~/.spoke/` and the managed blocks in `AGENTS.md` / `CLAUDE.md` (they are marked) |

---

## 6. Hardening checklist

- [ ] Set a conservative `umask` before first use so `~/.spoke/` (packages, backups) is not world-readable.
- [ ] Commit `agent.toml` and `agent.lock`; review changes to them like code.
- [ ] Read scan warnings on every `add`; treat any script file in a skill package as a red flag.
- [ ] Periodically prune `~/.spoke/backups/` — it grows forever and may contain secrets.
- [ ] Prefer `--dry-run` on any unfamiliar package.
- [ ] When a signed registry exists, prefer signed packages; until then, treat `integrity` as a consistency check only.

---

## 7. Reporting

Security issues in Spoke itself should be reported privately to the maintainer rather than in a public issue, with a minimal reproduction (Spoke version, the operation performed, and the affected path). Do not include secrets, tokens, or full configuration files in a report — paths and redacted snippets are enough.
