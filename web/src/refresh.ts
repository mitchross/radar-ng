/**
 * Runs `fn` every `periodMs` while the page is visible. A hidden tab skips
 * runs; when it returns to view after a full period it runs at once instead
 * of showing stale data until the next tick.
 */
export function startPeriodic(fn: () => void, periodMs: number, now: () => number = Date.now): () => void {
  let last = now();
  const run = () => {
    last = now();
    fn();
  };
  const timer = window.setInterval(() => {
    if (document.visibilityState === "visible") run();
  }, periodMs);
  const onVisible = () => {
    if (document.visibilityState === "visible" && now() - last >= periodMs) run();
  };
  document.addEventListener("visibilitychange", onVisible);
  return () => {
    window.clearInterval(timer);
    document.removeEventListener("visibilitychange", onVisible);
  };
}
