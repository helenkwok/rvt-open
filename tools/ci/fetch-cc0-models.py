#!/usr/bin/env python3
"""Fetch the CC0 models CI reads, from their publishers' own URLs.

Usage: fetch-cc0-models.py <pins.tsv> <target_dir>

The pins (research/cc0-models.tsv) give, per file, its name, the URL its
publisher serves it from, and its sha256 and byte count. Each file lands in
target_dir under its name and is checked against its pins; one already there
with the right hash is kept, so a run whose cache holds every file never
contacts the host. Nothing is stored in this repository.

Exit 0 when every pinned file is in place and verified; 1 when a download no
longer matches its pins (the publisher replaced the file: re-pin); 2 when a
host could not be reached after retries, which says nothing about the models.
"""

from __future__ import annotations

import hashlib
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

USER_AGENT = "rvt-open-ci (https://github.com/helenkwok/rvt-open)"
TRIES = 3


def read_pins(path: Path) -> list[tuple[str, str, str, int]]:
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        name, url, sha, size = line.split("\t")
        rows.append((name, url, sha.lower(), int(size)))
    return rows


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def download(url: str) -> bytes | None:
    for attempt in range(1, TRIES + 1):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(req, timeout=120) as r:
                return r.read()
        except (urllib.error.URLError, OSError) as e:
            print(f"{url}: attempt {attempt} failed: {e}", file=sys.stderr)
            if attempt < TRIES:
                time.sleep(5 * attempt)
    return None


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    pins, target = Path(sys.argv[1]), Path(sys.argv[2])
    target.mkdir(parents=True, exist_ok=True)
    status = 0
    for name, url, sha, size in read_pins(pins):
        dest = target / name
        if dest.is_file() and dest.stat().st_size == size and sha256(dest) == sha:
            print(f"{dest}: present and verified")
            continue
        data = download(url)
        if data is None:
            print(f"{name}: {url} could not be reached")
            status = max(status, 2)
            continue
        got = hashlib.sha256(data).hexdigest()
        if len(data) != size or got != sha:
            print(f"{name}: {len(data)} bytes, sha256 {got}; the pin is {size} bytes, {sha}")
            return 1
        tmp = dest.with_name(dest.name + ".part")
        tmp.write_bytes(data)
        tmp.replace(dest)
        print(f"{dest}: downloaded and verified")
    return status


if __name__ == "__main__":
    sys.exit(main())
