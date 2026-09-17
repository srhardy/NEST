#!/usr/bin/env python3
"""Write a nest notification for the watch remote.

usage: send.py "message" [tag]
Extra targets go in ~/.nest/remotes/watch.toml as [[targets]] url="..." —
each gets the message POSTed as JSON {"text": ...} (e.g. a gadget relay).
"""
import json
import re
import sys
import time
import urllib.request
from pathlib import Path

HOME = Path.home() / ".nest" / "remotes"
OUTBOX = HOME / "watch" / "outbox"
CFG = HOME / "watch.toml"


def main():
    text = sys.argv[1] if len(sys.argv) > 1 else "ping from the nest"
    tag = sys.argv[2] if len(sys.argv) > 2 else "note"
    OUTBOX.mkdir(parents=True, exist_ok=True)
    ts = int(time.time())
    (OUTBOX / f"{ts}-{tag}.txt").write_text(text)
    if CFG.exists():
        for url in re.findall(r'url\s*=\s*"([^"]+)"', CFG.read_text()):
            try:
                req = urllib.request.Request(
                    url, data=json.dumps({"text": text, "tag": tag, "ts": ts}).encode(),
                    headers={"Content-Type": "application/json"},
                )
                urllib.request.urlopen(req, timeout=5).read()
            except Exception as e:
                print(f"target {url}: {e}", file=sys.stderr)
    print(f"sent: {OUTBOX / f'{ts}-{tag}.txt'}")


if __name__ == "__main__":
    main()
