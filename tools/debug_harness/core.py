"""Shared plumbing: timed HTTP fetches, percentiles, check results, rendering.

No third-party imports — this must run from a bare python:3.12 container or a
laptop without the repo's requirements installed.
"""

from __future__ import annotations

import json
import ssl
import statistics
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

DEFAULT_TIMEOUT_S = 15.0

# Severity ladder for check results. `fail` flips the process exit code.
OK, WARN, FAIL = "ok", "warn", "fail"
_ICONS = {OK: "✓", WARN: "⚠", FAIL: "✗"}
_ORDER = {OK: 0, WARN: 1, FAIL: 2}


@dataclass
class Fetch:
    """One timed HTTP GET. `error` is set instead of raising so samplers can
    aggregate failures into rates rather than aborting the run."""

    url: str
    status: int = 0
    elapsed_ms: float = 0.0
    bytes: int = 0
    headers: dict[str, str] = field(default_factory=dict)
    body: bytes = b""
    error: str | None = None

    @property
    def ok(self) -> bool:
        return self.error is None and 200 <= self.status < 400


@dataclass
class Check:
    name: str
    status: str  # ok | warn | fail
    detail: str
    data: dict[str, Any] = field(default_factory=dict)


def fetch(url: str, *, timeout: float = DEFAULT_TIMEOUT_S, read_body: bool = True) -> Fetch:
    req = urllib.request.Request(url, headers={"User-Agent": "radar-ng-debug-harness"})
    # Self-hosted stacks routinely use self-signed certs; the harness reads
    # public weather tiles, so unverified TLS is an acceptable trade here.
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    start = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout, context=ctx) as resp:
            body = resp.read() if read_body else b""
            elapsed = (time.perf_counter() - start) * 1000
            return Fetch(
                url=url,
                status=resp.status,
                elapsed_ms=elapsed,
                bytes=len(body),
                headers={k.lower(): v for k, v in resp.headers.items()},
                body=body,
            )
    except urllib.error.HTTPError as exc:
        elapsed = (time.perf_counter() - start) * 1000
        return Fetch(
            url=url,
            status=exc.code,
            elapsed_ms=elapsed,
            headers={k.lower(): v for k, v in exc.headers.items()} if exc.headers else {},
            error=f"HTTP {exc.code}",
        )
    except Exception as exc:  # URLError, timeout, ConnectionReset, ...
        elapsed = (time.perf_counter() - start) * 1000
        return Fetch(url=url, elapsed_ms=elapsed, error=f"{type(exc).__name__}: {exc}")


def fetch_json(url: str, *, timeout: float = DEFAULT_TIMEOUT_S) -> tuple[Fetch, Any]:
    f = fetch(url, timeout=timeout)
    if not f.ok:
        return f, None
    try:
        return f, json.loads(f.body)
    except json.JSONDecodeError as exc:
        f.error = f"invalid JSON: {exc}"
        return f, None


def percentiles(samples_ms: list[float]) -> dict[str, float]:
    if not samples_ms:
        return {}
    s = sorted(samples_ms)
    def pct(p: float) -> float:
        idx = min(len(s) - 1, max(0, round(p / 100 * (len(s) - 1))))
        return s[idx]
    return {
        "n": len(s),
        "min_ms": round(s[0], 1),
        "p50_ms": round(statistics.median(s), 1),
        "p95_ms": round(pct(95), 1),
        "max_ms": round(s[-1], 1),
    }


def parse_iso(ts: str) -> datetime | None:
    try:
        dt = datetime.fromisoformat(ts)
    except ValueError:
        return None
    # Ingest writes tz-aware isoformat, but tolerate naive strings as UTC.
    return dt if dt.tzinfo else dt.replace(tzinfo=timezone.utc)


def utcnow() -> datetime:
    return datetime.now(timezone.utc)


def age_str(seconds: float) -> str:
    seconds = int(seconds)
    if abs(seconds) < 120:
        return f"{seconds}s"
    if abs(seconds) < 7200:
        return f"{seconds // 60}m{abs(seconds) % 60:02d}s"
    return f"{seconds // 3600}h{(abs(seconds) % 3600) // 60:02d}m"


def quote_ts(timestamp: str) -> str:
    """Timestamp dirnames carry ':' and '+' — encode them for tile URLs the
    same way the app does, so the harness exercises the real request shape."""
    return urllib.parse.quote(timestamp, safe="")


# ---------- rendering ----------


def worst(checks: list[Check]) -> str:
    return max((c.status for c in checks), key=lambda s: _ORDER[s], default=OK)


def render_checks(title: str, checks: list[Check], *, as_json: bool) -> None:
    if as_json:
        print(json.dumps(
            {
                "section": title,
                "status": worst(checks),
                "checked_at": utcnow().isoformat(),
                "checks": [c.__dict__ for c in checks],
            },
            indent=2,
            default=str,
        ))
        return
    print(f"\n== {title} ==")
    for c in checks:
        print(f"  {_ICONS[c.status]} {c.name}: {c.detail}")


def exit_code(checks: list[Check]) -> int:
    return 1 if worst(checks) == FAIL else 0


def eprint(*args: Any) -> None:
    print(*args, file=sys.stderr)
