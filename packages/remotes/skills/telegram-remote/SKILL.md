---
name: telegram-remote
description: Talk to the nest from Telegram — a polling bridge that turns messages into inbox files and sends outbox replies back. No framework, stdlib only.
---

# Telegram remote

A two-folder bridge between Telegram and the nest:

- `~/.nest/remotes/telegram/inbox/` — one `.md` file per incoming message; the
  boss reads these as tasks.
- `~/.nest/remotes/telegram/outbox/` — drop a `<chat_id>.txt` file here and the
  bridge sends it back to that chat, then deletes it.

## Setup

1. Message `@BotFather` on Telegram → `/newbot` → copy the token.
2. Write `~/.nest/remotes/telegram.toml`:

```toml
token = "123456:ABC-your-bot-token"
# only these chats may drive the nest (empty = any)
chats = [123456789]
```

3. Declare the service in `agent.toml` so it starts with the nest:

```toml
[[services]]
name = "telegram-remote"
start_cmd = "python3 ~/.nest/skills/telegram-remote/bridge.py"
```

4. `spoke sync`, then start the service. The bridge polls `getUpdates` every
   2 s and writes each message to the inbox; replies go in the outbox.

## What the bridge does NOT do

No secrets in logs, no commands executed — it only moves text between Telegram
and files. The boss decides what an inbox file means.
