#!/usr/bin/env python3
"""Verify a device profile before building Radar NG's CarPlay variant."""
import argparse
import plistlib
import subprocess
from datetime import datetime, timezone
from pathlib import Path


def validate(profile, bundle_id, team_id):
    entitlement = profile.get("Entitlements", {})
    expected = f"{team_id}.{bundle_id}"
    errors = []
    if entitlement.get("application-identifier") != expected:
        errors.append(f"Profile must be for {expected}")
    if entitlement.get("com.apple.developer.carplay-maps") is not True:
        errors.append("Profile does not contain Apple's CarPlay navigation capability")
    expires = profile.get("ExpirationDate")
    if not expires or expires.replace(tzinfo=timezone.utc) <= datetime.now(timezone.utc):
        errors.append("Profile is expired or has no expiration date")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", type=Path)
    parser.add_argument("--bundle-id", default="com.vanillax.radar-ng")
    parser.add_argument("--team-id", default="FVU6RGL532")
    args = parser.parse_args()
    result = subprocess.run(["security", "cms", "-D", "-i", str(args.profile)],
                            capture_output=True, check=True)
    errors = validate(plistlib.loads(result.stdout), args.bundle_id, args.team_id)
    if errors:
        raise SystemExit("CarPlay profile not ready:\n- " + "\n- ".join(errors))
    print("PASS: unexpired Radar NG profile includes CarPlay navigation")


if __name__ == "__main__":
    main()
