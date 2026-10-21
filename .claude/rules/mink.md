---
description: Mink context management — automatic via hooks
---

> **Only applies if the `mink` CLI is installed** (`command -v mink`). If it isn't, ignore this file:
> the hooks in `.claude/settings.json` turn themselves off, and nothing else depends on Mink.

This project's maintainer uses **Mink** (`@drewpayment/mink`) for cross-session context management.

## How it works
- Mink runs automatically through Claude Code hooks configured in `.claude/settings.json` (SessionStart, PreToolUse, PostToolUse, Stop).
- All state lives in `~/.mink/` on the user's machine — **not** in this repository. Do not create or write to any in-repo state directory (no `.wolf/`, `.mink/`, etc.).
- Read intelligence, write enforcement, bug memory, and the token ledger are handled by the hooks. You do not need to manually read or update any state files.

## When to act on Mink
- If the user asks to "save a note", "remember this", "log this to my wiki", or similar, use the `mink-note` skill — it captures into the user's `~/.mink/` vault.
- If a hook surfaces a learning, past bug, or repeat-read warning, treat that as authoritative project memory and follow it.
- The `mink dashboard` and `mink agent` commands are user tools — do not invoke them on the user's behalf.
