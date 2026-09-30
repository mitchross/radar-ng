# Agent Instructions

## Mink Knowledge Capture

Keep Mink updated during substantive work. Hooks may track session state automatically, but durable decisions, verified root causes, runbooks, and gotchas require explicit note capture with `mink note` or `/mink:note`.

Use `mink note --project radar-ng --category resources` for durable references and `--category projects` for active decisions or followups. Do not capture routine edits, raw command output, or unverified hypotheses. Mention saved Mink note paths in the final response.

## Web + app parity

Every user-facing feature ships to **both** the web radar (`web/`, radar.vanillax.me) and the Expo app (`frontend/`) in the same change — never one without the other. When planning, list the web piece, the app piece, and what is server-side (automatic for both). App changes reach the phone only after a native rebuild (`bun run ios:ipa`); say so when handing off.
