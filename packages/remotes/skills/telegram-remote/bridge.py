#!/usr/bin/env python3
"""Telegram <-> nest remote bridge (stdlib only).

Polls getUpdates into ~/.nest/remotes/telegram/inbox/*.md and flushes
outbox/<chat_id>.txt files back to Telegram. Config: ~/.nest/remotes/telegram.toml
"""
import json
import re
import sys
import time
import urllib.request
from pathlib import Path

HOME = Path.home() / ".nest" / "remotes"
INBOX = HOME / "telegram" / "inbox"
OUTBOX = HOME / "telegram" / "outbox"
OFFSET = HOME / "telegram" / "offset"


def cfg():
    text = (HOME / "telegram.toml").read_text()
    token = re.search(r'^token\s*=\s*"([^"]+)"', text, re.M).group(1)
    chats = re.findall(r"\d+", re.search(r"^chats\s*=\s*\[([^\]]*)\]", text, re.M).group(1)) \
        if re.search(r"^chats\s*=", text, re.M) else []
    return token, set(chats)


def api(token, method, **kw):
    data = json.dumps(kw).encode()
    req = urllib.request.Request(
        f"https://api.telegram.org/bot{token}/{method}",
        data=data, headers={"Content-Type": "application/json"},
    )
    return json.load(urllib.request.urlopen(req, timeout=30))


def main():
    token, allowed = cfg()
    INBOX.mkdir(parents=True, exist_ok=True)
    OUTBOX.mkdir(parents=True, exist_ok=True)
    offset = int(OFFSET.read_text().strip() or 0) if OFFSET.exists() else 0
    while True:
        try:
            r = api(token, "getUpdates", offset=offset, timeout=25)
            for u in r.get("result", []):
                offset = u["update_id"] + 1
                OFFSET.write_text(str(offset))
                msg = u.get("message", {})
                text, chat = msg.get("text", ""), str(msg.get("chat", {}).get("id", ""))
                if not text or (allowed and chat not in allowed):
                    continue
                who = msg.get("from", {}).get("username") or msg.get("from", {}).get("first_name", "?")
                (INBOX / f"{msg['message_id']}-{chat}.md").write_text(
                    f"---\nremote: telegram\nchat: {chat}\nfrom: {who}\n---\n\n{text}\n"
                )
            for f in OUTBOX.glob("*.txt"):
                api(token, "sendMessage", chat_id=int(f.stem), text=f.read_text()[:4000])
                f.unlink()
            time.sleep(2)
        except KeyboardInterrupt:
            return
        except Exception as e:  # network hiccup — keep bridging
            print(f"bridge: {e}", file=sys.stderr)
            time.sleep(5)


if __name__ == "__main__":
    main()
