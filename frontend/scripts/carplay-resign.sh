#!/usr/bin/env bash
# Retained to give users of the old command an actionable correction.
set -euo pipefail
cat >&2 <<'MESSAGE'
The old post-archive CarPlay entitlement bypass was incorrect.
iOS checks signed entitlements against the provisioning profile; Developer Mode
and a paid account do not bypass that check. This script makes no changes.

For a real car, request the appropriate CarPlay entitlement from Apple and
regenerate the development provisioning profile. For simulator development,
see docs/carplay-watch-setup.md. Small widgets can appear in CarPlay on iOS 26+
without making the containing app a full CarPlay app.
MESSAGE
exit 1
