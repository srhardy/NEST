---
name: slack-remote
description: Talk to the nest from Slack — a polling bridge using the Slack Web API (no sockets, stdlib only). Mentions/DMs become inbox files; outbox replies post back.
---

# Slack remote

Same two-folder pattern as the other remotes:

- `~/.nest/remotes/slack/inbox/` — one `.md` per message addressed to the bot.
- `~/.nest/remotes/slack/outbox/` — `<channel>.txt` files get posted back.

## Setup

1. api.slack.com/apps → Create App → Bot scopes: `channels:history`,
   `chat:write`, `im:history`, `app_mentions:read`. Install to workspace,
   copy the `xoxb-` token.
2. Write `~/.nest/remotes/slack.toml`:

```toml
token = "xoxb-your-bot-token"
# channels to watch (empty = app mentions in every joined channel)
channels = ["C0123ABC"]
```

3. Declare the service in `agent.toml`:

```toml
[[services]]
name = "slack-remote"
start_cmd = "python3 ~/.nest/skills/slack-remote/bridge.py"
```

The bridge polls `conversations.history` per channel every 5 s. Invite the bot
(`@nest`) to any channel you want watched.
