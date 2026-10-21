#!/usr/bin/env python3
"""Fail if llms.txt or any maintained markdown file links to a repo file that doesn't exist.

Checks:
- llms.txt: every https://github.com/<owner>/radar-ng/blob|tree/master/<path> link (any owner, so
  forks pass too) must exist in the checkout.
- *.md outside docs/archive and node_modules: every relative link target must exist
  (anchors and external URLs are ignored).
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SKIP = ("node_modules", ".git", "docs/archive", "target", "/ios/", "/android/")
errors = []

gh = re.compile(r"https://github\.com/[^/\s]+/radar-ng/(?:blob|tree)/master/([^)\s#]+)")
for path in gh.findall((ROOT / "llms.txt").read_text()):
    if not (ROOT / path).exists():
        errors.append(f"llms.txt -> {path}")

link = re.compile(r"(?<!!)\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
fence = re.compile(r"```.*?```", re.S)
for md in ROOT.rglob("*.md"):
    rel = md.relative_to(ROOT).as_posix()
    if any(s.strip("/") in rel for s in SKIP):
        continue
    text = fence.sub("", md.read_text(errors="ignore"))
    for target in link.findall(text):
        if re.match(r"^[a-z][a-z0-9+.-]*:", target, re.I) or target.startswith("#"):
            continue
        target = target.split("#", 1)[0]
        if not target:
            continue
        resolved = (ROOT / target.lstrip("/")) if target.startswith("/") else (md.parent / target)
        if not resolved.exists():
            errors.append(f"{rel} -> {target}")

for e in errors:
    print(f"BROKEN {e}")
print(f"{len(errors)} broken link(s)")
sys.exit(1 if errors else 0)
