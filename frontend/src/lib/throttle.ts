/**
 * At most one call per `ms`: the first call runs at once, later ones within
 * the interval collapse into a single trailing call with the latest value.
 * `flush` runs a value immediately (e.g. when a drag ends).
 */
export function createThrottle<T>(fn: (value: T) => void, ms: number, now: () => number = Date.now) {
  let last = -Infinity;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let pending: { value: T } | undefined;

  const clear = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };
  const fire = (value: T) => {
    clear();
    pending = undefined;
    last = now();
    fn(value);
  };

  return {
    call(value: T) {
      const wait = last + ms - now();
      if (wait <= 0) {
        fire(value);
        return;
      }
      pending = { value };
      timer ??= setTimeout(() => {
        timer = undefined;
        if (pending) fire(pending.value);
      }, wait);
    },
    flush: fire,
    cancel() {
      clear();
      pending = undefined;
    },
  };
}
