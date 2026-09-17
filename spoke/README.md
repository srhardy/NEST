# Spoke

One hub for every coding agent. One manifest (`agent.toml`), one lockfile, one event
log — projected into every harness you use.

Status: M0 (Sync spoke). Claude Code, Codex and Nest adapters; `init`, `sync`, `diff`, `doctor`.

## Layout

```
crates/spoke-core       manifest, projection IR, lockfile, paths
crates/spoke-adapters   per-harness adapters + safe apply/backup engine
crates/spoke-registry   package resolution, fetching, scanning, dedupe
crates/spoke-cli        the `spoke` binary
docs/                   ABOUT · MANIFEST · ADAPTER-CONTRACT · HARNESS-MATRIX · SECURITY
```

## Docs

| Doc | Read it for |
|---|---|
| [docs/ABOUT.md](docs/ABOUT.md) | Why Spoke exists, what it solves, the design decisions and their justifications (TL;DR at the top) |
| [docs/MANIFEST.md](docs/MANIFEST.md) | Every file format: `agent.toml`, `package.toml`, `index.toml`, `agent.lock`, the `~/.spoke` layout |
| [docs/ADAPTER-CONTRACT.md](docs/ADAPTER-CONTRACT.md) | How to add a harness: the trait, the operation catalogue, the rules, the required tests |
| [docs/HARNESS-MATRIX.md](docs/HARNESS-MATRIX.md) | What each harness accepts, what Spoke writes today, known gaps and trust gates |
| [docs/SECURITY.md](docs/SECURITY.md) | What Spoke touches, the threat model, package trust limits, the reversibility runbook |

## Design rules

- **Never clobber user config.** Spoke writes either a marked block
  (`<!-- spoke:begin ID --> … <!-- spoke:end ID -->`) into instruction files, or
  performs surgical JSON/TOML merges that preserve unknown keys and formatting.
- **Backups before every write.** `~/.spoke/backups/<unix-seconds>/…`.
- **Warn, don't silently drop.** If a harness cannot express something, it appears in
  `spoke doctor` / `spoke sync` warnings.
- **Idempotent.** Applying twice changes nothing the second time.

## Usage

```sh
spoke init                  # detect harnesses, write agent.toml, prepare backups
spoke diff                  # show what would change
spoke sync                  # apply, with backups
spoke doctor                # harness detection, capabilities, drift, warnings
spoke --home /tmp/h --project /tmp/p sync   # explicit paths (tests, VPS)
```

## License

Apache-2.0.
