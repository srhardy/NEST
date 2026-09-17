#!/usr/bin/env python3
"""Slack <-> nest remote bridge (stdlib only, Web API polling).

New channel messages land in ~/.nest/remotes/slack/inbox/*.md and
outbox/<channel>.txt files get posted back. Config: ~/.nest/remotes/slack.toml
"""
import json
import re
import sys
import time
import urllib.request
from pathlib import Path

HOME = Path.home() / ".nest" / "remotes"
INBOX = HOME / "slack" / "inbox"
OUTBOX = HOME / "slack" / "outbox"
CURSOR = HOME / "slack" / "cursors.json"


def cfg():
    text = (HOME / "slack.toml").read_text()
    token = re.search(r'^token\s*=\s*"([^"]+)"', text, re.M).group(1)
    m = re.search(r"^channels\s*=\s*\[([^\]]*)\]", text, re.M)
    channels = re.findall(r'"([^"]+)"', m.group(1)) if m else []
    return token, channels


def api(token, method, **kw):
    req = urllib.request.Request(
        f"https://slack.com/api/{method}",
        data=json.dumps(kw).encode(),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"},
    )
    return json.load(urllib.request.urlopen(req, timeout=30))


def main():
    token, channels = cfg()
    INBOX.mkdir(parents=True, exist_ok=True)
    OUTBOX.mkdir(parents=True, exist_ok=True)
    cursors = json.loads(CURSOR.read_text()) if CURSOR.exists() else {}
    me = api(token, "auth.test").get("user_id", "")
    while True:
        try:
            for ch in channels:
                kw = {"channel": ch, "limit": 20}
                if cursors.get(ch):
                    kw["oldest"] = cursors[ch]
                r = api(token, "conversations.history", **kw)
                for m in r.get("messages", []):
                    cursors[ch] = m["ts"]
                    if m.get("subtype") or m.get("user") == me:
                        continue
                    text = m.get("text", "")
                    if me and f"<@{me}>" not in text:
                        continue
                    (INBOX / f"{m['ts'].replace('.', '_')}-{ch}.md").write_text(
                        f"---\nremote: slack\nchannel: {ch}\nfrom: {m.get('user','?')}\n---\n\n{text}\n"
                    )
            CURSOR.write_text(json.dumps(cursors))
            for f in OUTBOX.glob("*.txt"):
                api(token, "chat.postMessage", channel=f.stem, text=f.read_text()[:3900])
                f.unlink()
            time.sleep(5)
        except KeyboardInterrupt:
            return
        except Exception as e:
            print(f"bridge: {e}", file=sys.stderr)
            time.sleep(10)


if __name__ == "__main__":
    main()
