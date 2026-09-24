import { useEffect, useState } from "react";

/**
 * Wall-clock milliseconds, refreshed every `intervalMs` while mounted. Keeps
 * `Date.now()` out of render, where React Compiler could memoize a stale value.
 */
export function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return now;
}
