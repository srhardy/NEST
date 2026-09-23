# Phase 5 design — cloud tier on the smallest possible backend

Status: proposal. Everything here builds on what already exists in `spoke/`
and `nest/`; the only genuinely new code is a `spoke registry sync` command
and one webhook bridge. No bespoke server is required for v0.

## What already exists (and is often mistaken for missing)

| Piece | Where | State |
|---|---|---|
| Package index | `~/.spoke/registry/index.toml` | shipped — `Store::resolve` maps a package name to a manifest path |
| Remote package sources | `manifest.source.git` | shipped — `Store::fetch` already does `git clone --depth 1` (optional `rev`) into `~/.spoke/packages/` |
| External triggers (local) | `~/.nest/remotes/<name>/inbox/*.md` | shipped — `remotes_watch` brood-dispatches each file and writes replies to `outbox/`; starter bridges exist (`packages/remotes/skills/{slack,telegram,watch}-remote`) |
| In-app webhook | `POST /api/duties/<name>/fire` | shipped — settings "fire" button and external callers share one dispatch path |
| Headless runtime | `nest daemon` | shipped — patrol + remotes loop, no HTTP listener, launchd/systemd unit |

## The actual gap

1. The registry index is **local-only** — there's no command to pull a shared
   index, so `spoke add <name>` only works for packages already on disk.
2. The remotes inbox is a **filesystem contract** — reachable only from the
   same machine. Telegram/Slack bridges poll their provider and drop files;
   nothing inbound crosses the network.
3. Auth/rate-limiting/multi-user semantics — undefined (and unneeded until
   something listens on a socket).

## Proposal v0 — the registry is a git repo

GitHub *is* the backend. A public repo `srhardy/nest-registry` holds:

```
index.toml                 # [[package]] name, version, manifest = "pkg/<name>/package.toml"
pkg/<name>/package.toml    # [source] git = "<clone-url>", rev = "<tag>"
```

New command:

```
spoke registry sync [<git-url>]   # clone or pull into ~/.spoke/registry/
spoke registry show               # optional: pretty-print the index
```

That's it — `spoke add <name>` then resolves through the synced index and
`fetch` clones each package's own source repo. Integrity: pin `rev` to a tag
or sha in the manifest; `spoke why <name>` already surfaces the resolved
source. Publishing is a PR on the registry repo — review is the approval
workflow, free.

Index semantics: one entry per name — `[[package]]` maps a name to exactly
one manifest and `version` in the index is advisory only; the version that
actually installs comes from the manifest (and lands in `agent.lock`).
Multiple versions of the same name need distinct entries
(`foo-1x`, `foo-2x`) or a later index schema change — deferred.

Why this beats a hosted API for v0: zero infra to run, versioned by git,
works with private repos via the user's existing git credentials, and the
`Index`/`Store` code path doesn't change — only a sync wrapper does. If a
real service is ever needed, the same `index.toml` + `source.git` contract
sits behind it unchanged.

## External triggers — extend the inbox contract, don't replace it

The daemon already owns the dispatch loop; the bridge is the only missing
piece. `packages/remotes/skills/*-remote` establishes the pattern: a bridge
is a small process that translates one provider into inbox files. Guards on
the listener: bodies cap at 64KiB, a remote's backlog at 64 pending tasks
(both refused with 413/429 rather than filling disk or flooding dispatch).
A captured bearer token stays replayable by design of v0 — mitigate by
rotating `token` files; request-signing (HMAC over body+timestamp) is the
listed hardening if real adversaries show up. For
inbound-over-internet:

```
POST /inbox/<remote>   (Authorization: Bearer <token>)
  → writes ~/.nest/remotes/<remote>/inbox/<ts>.md
  → existing remotes_watch dispatches → outbox/<chat>.txt
     (no chat:/channel: line → replies under the task's own filename,
      outbox/<ts>.txt, retrievable at GET /inbox/<remote>/<ts>/reply)
```

Where it runs: beside `nest daemon` on a VPS, or on a laptop behind
`cloudflared`/`tailscale funnel` — the webhook listener is a ~100-line
binary (or a tiny adapter in `nest daemon` itself behind a flag) reading the
token from `~/.nest/remotes/<remote>/token`. HTTPS comes from the tunnel; no
TLS code in-process. This keeps every reply path identical to today's
bridges and adds exactly one auth check.

Non-goal for v0: multi-tenant hosting. `nest daemon` stays single-user; the
cloud tier shares *packages and triggers*, not runtime.

## Sequencing

1. `spoke registry sync/show` + a seeded `nest-registry` repo — seeding
   means vendoring each package's *contents* under `pkg/<name>/` (manifest
   `source.path = "."` resolves relative to the manifest's own directory,
   so copying the manifest alone would point the source at nothing), or
   giving each package its own `source.git` repo. ~1 session, no infra.
2. Webhook-inbox bridge + `token` file convention + a `cloudflared` recipe
   in docs — ~1 session.
3. Only if adoption demands: a real index service mirroring `index.toml`
   semantics — punt.

## Open questions for Steven

- Registry repo name/location (`srhardy/nest-registry`? inside this repo's
  `registry/` dir instead — a `git subtree` pull would work too)?
- Pin package `rev`s (immutable, safer) or track `main` (fresher, riskier)?
- Webhook bridge: separate tiny binary, or a `--webhook` flag on `nest daemon`?
