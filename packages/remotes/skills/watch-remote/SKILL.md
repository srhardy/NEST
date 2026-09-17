---
name: watch-remote
description: The smart-watch remote — a plain spool folder a watch (or anything else) can read and write. Notifications land in outbox/; dropped files in inbox/ become tasks.
---

# Watch remote

The simplest remote: **no bridge at all**. A smart watch (or any gadget,
shortcut, or cron job) only needs to read and write two folders:

- `~/.nest/remotes/watch/outbox/` — the nest drops `<ts>-<tag>.txt`
  notifications here; the watch polls the folder (or a sync app mirrors it)
  and shows them.
- `~/.nest/remotes/watch/inbox/` — drop a `.md` file and the nest treats it as
  a task.

## What to install on the watch side

Anything that can sync a folder: Syncthing, a Shortcuts automation, a tiny
shell script on a WearOS/PineTime companion. NEST's side is just the folders —
the skill ships a `send.py` helper the boss calls to write a notification:

```bash
python3 ~/.nest/skills/watch-remote/send.py "build finished" watch
```

That writes `~/.nest/remotes/watch/outbox/<ts>-watch.txt`. If your gadget uses
a different transport (an HTTP endpoint, a BLE relay), point `send.py` at it —
it's 20 lines of urllib.
