# Installer findings — CommandCodeAI/desktop

Reviewed `install.sh` (325 lines) and `install.ps1` (160 lines) on `main`
(HEAD `2ace01b`).

## BUG (ship-blocker): `install.sh` can never match a release asset

`read_asset_metadata()` gates asset extraction on this awk pattern
(`install.sh` line 52):

```awk
/"url": "https:\/\/api.github.com\/repos\/CommandCodeAI\/gui\/releases\/assets\// {
```

But releases live on `CommandCodeAI/desktop`, and the live GitHub API response
contains **zero** occurrences of `CommandCodeAI/gui` — every asset `url` is
`https://api.github.com/repos/CommandCodeAI/desktop/releases/assets/...`
(verified against `v0.1.30` release JSON).

Consequence: `in_asset` never becomes true → `asset_metadata` is always empty
→ every platform exits with:

```
Command Code installer: No verified macOS package was found in the 20 newest releases.
```

**The macOS and Linux installer is broken on main for all users.**

Root cause is visible in history: commit `acc1293` "fix: gui → desktop"
renamed the repo references but this literal string inside the awk program
survived (or was re-introduced).

### Suggested fix

Match assets by name without depending on the API `url` field at all —
the `name`/`digest`/`browser_download_url` fields are sufficient:

```awk
/"name": "/    { ... asset_name = value }
/"digest": "sha256:/ { ... digest = value }
/"browser_download_url":/ {
    if (asset_name == wanted) { print digest; print value; exit }
    asset_name = ""; digest = ""
}
```

or simply replace `CommandCodeAI/gui` → `CommandCodeAI/desktop` (minimal fix).
Add a CI smoke test: run `install.sh` with
`COMMANDCODE_INSTALL_VERIFY_ONLY=1` on each OS runner — it would have caught
this immediately.

## Minor issues

1. **Stale error message** — `install.sh:232` still fails `Darwin:*` with
   "The current macOS preview supports Apple silicon only", but Intel
   (`x86_64`) support was added in commit `94cfece`. Harmless (only reached on
   non-x86_64/arm64 Macs) but the message is wrong.
2. **Fragile tag parsing** — `install.sh:261` extracts `tag_name` with
   `awk -F'"' '/"tag_name":/'`, which depends on GitHub's JSON pretty-print
   layout. Works today; a `python3`/`jq` fallback or grep on
   `"tag_name": "v..."` is more robust.
3. **`install.ps1` is not affected** — it matches on `asset.name`, not the
   URL, and correctly checks `digest`. No action needed.
4. Windows Authenticode is still unsigned (expected per `INSTALL.md` — the
   script's prompt path handles it).

## Verified-good installer properties (worth keeping)

- SHA-256 digest verification against GitHub-published digests — both scripts.
- macOS: Developer ID signature check + `spctl` Gatekeeper assessment, fails
  closed, never strips quarantine. Good.
- Backup/restore of existing `/Applications/Command Code.app` on copy failure.
- `COMMANDCODE_INSTALL_DRY_RUN` / `_VERIFY_ONLY` env hooks — extend these into
  CI to regression-test resolution.
- No execution of downloaded content before verification; `.deb` installed via
  `apt-get` rather than piping a payload to a shell.
