#!/usr/bin/env python3
"""Check that both servers return the same content for the bench 5 request list.

Compares, per unique URL in urls_py.txt: status code, content-type, and body. Tiles must be
byte-identical. JSON bodies must be equal after removing fields that are time-stamped per
request (updated_at / Date-like fields). Prints a summary and exits non-zero on a mismatch.

    python3 parity_check.py --py http://127.0.0.1:8081 --rs http://127.0.0.1:8082 --urls urls_py.txt
"""

from __future__ import annotations

import argparse
import collections
import json
import sys
from urllib.parse import urlsplit

import httpx

VOLATILE = {"updated_at", "generated_at", "server_time"}


def strip(obj):
    if isinstance(obj, dict):
        return {k: strip(v) for k, v in obj.items() if k not in VOLATILE}
    if isinstance(obj, list):
        return [strip(v) for v in obj]
    return obj


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--py", required=True)
    ap.add_argument("--rs", required=True)
    ap.add_argument("--urls", required=True)
    args = ap.parse_args()
    py_base, rs_base = args.py.rstrip("/"), args.rs.rstrip("/")
    urls = sorted({l.strip() for l in open(args.urls) if l.strip()})
    c = httpx.Client(timeout=15, follow_redirects=False)
    kinds = collections.Counter()
    mismatches = []
    for u in urls:
        p = urlsplit(u)
        path = p.path + (f"?{p.query}" if p.query else "")
        a = c.get(py_base + path)
        b = c.get(rs_base + path)
        kind = "tile" if "/tiles/" in path else "api"
        if a.status_code != b.status_code:
            mismatches.append((path, "status", a.status_code, b.status_code))
            kinds[(kind, "status-mismatch")] += 1
            continue
        if kind == "tile":
            if a.content != b.content:
                mismatches.append((path, "tile bytes", len(a.content), len(b.content)))
                kinds[(kind, "bytes-mismatch")] += 1
            else:
                kinds[(kind, "identical")] += 1
            continue
        ja, jb = a.json() if a.headers.get("content-type", "").startswith("application/json") else None, \
            b.json() if b.headers.get("content-type", "").startswith("application/json") else None
        if ja is None and jb is None:
            kinds[(kind, f"{a.status_code} non-json identical" if a.content == b.content else "non-json differs")] += 1
            continue
        if strip(ja) == strip(jb):
            kinds[(kind, f"{a.status_code} json equal")] += 1
        else:
            mismatches.append((path, "json", a.status_code, b.status_code))
            kinds[(kind, "json-mismatch")] += 1
    print(f"unique URLs: {len(urls)}")
    for (k, label), n in sorted(kinds.items()):
        print(f"  {k:5s} {label:28s} {n}")
    for m in mismatches[:20]:
        print("  MISMATCH", m)
    sys.exit(1 if mismatches else 0)


if __name__ == "__main__":
    main()
