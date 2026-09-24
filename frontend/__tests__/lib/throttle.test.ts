import { createThrottle } from "../../src/lib/throttle";

describe("createThrottle", () => {
  let clock = 0;
  const now = () => clock;

  beforeEach(() => {
    jest.useFakeTimers();
    clock = 0;
  });
  afterEach(() => jest.useRealTimers());

  function advance(ms: number) {
    clock += ms;
    jest.advanceTimersByTime(ms);
  }

  it("runs the first call at once and collapses the rest into one trailing call", () => {
    const calls: number[] = [];
    const t = createThrottle((v: number) => calls.push(v), 100, now);
    t.call(1);
    advance(20);
    t.call(2);
    advance(20);
    t.call(3);
    expect(calls).toEqual([1]);
    advance(60);
    expect(calls).toEqual([1, 3]);
  });

  it("flush commits the final value now and drops the pending one", () => {
    const calls: number[] = [];
    const t = createThrottle((v: number) => calls.push(v), 100, now);
    t.call(1);
    advance(10);
    t.call(2);
    t.flush(7);
    advance(200);
    expect(calls).toEqual([1, 7]);
  });

  it("cancel drops a pending call", () => {
    const calls: number[] = [];
    const t = createThrottle((v: number) => calls.push(v), 100, now);
    t.call(1);
    t.call(2);
    t.cancel();
    advance(200);
    expect(calls).toEqual([1]);
  });
});
