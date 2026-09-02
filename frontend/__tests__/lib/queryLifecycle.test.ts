import {
  QueryClient,
  QueryObserver,
  onlineManager,
} from "@tanstack/react-query";
import {
  bindAppFocus,
  bindNetworkOnline,
  runOnlineRefresh,
} from "../../src/lib/queryLifecycle";

describe("native Query lifecycle", () => {
  afterEach(() => {
    onlineManager.setOnline(true);
  });

  it("publishes initial AppState focus, follows changes, and unsubscribes", () => {
    let listener: ((state: string) => void) | undefined;
    const remove = jest.fn();
    const setFocused = jest.fn();
    const appState = {
      currentState: "background",
      addEventListener: jest.fn((_event: "change", nextListener: (state: string) => void) => {
        listener = nextListener;
        return { remove };
      }),
    };

    const unsubscribe = bindAppFocus(appState, setFocused);
    listener?.("active");
    unsubscribe();

    expect(setFocused.mock.calls).toEqual([[false], [true]]);
    expect(appState.addEventListener).toHaveBeenCalledWith("change", expect.any(Function));
    expect(remove).toHaveBeenCalledTimes(1);
  });

  it("uses link connectivity, not public-internet reachability, and unsubscribes", () => {
    let listener: ((state: { isConnected: boolean | null }) => void) | undefined;
    const unsubscribe = jest.fn();
    const setOnline = jest.fn();
    const connectivity = {
      addEventListener: jest.fn((nextListener: typeof listener) => {
        listener = nextListener;
        return unsubscribe;
      }),
    };

    const cleanup = bindNetworkOnline(connectivity, setOnline);
    listener?.({ isConnected: false });
    listener?.({ isConnected: true });
    listener?.({ isConnected: null });
    cleanup();

    expect(setOnline.mock.calls).toEqual([[false], [true], [true]]);
    expect(unsubscribe).toHaveBeenCalledTimes(1);
  });

  it("does not start an awaited manual refresh while offline", async () => {
    const refresh = jest.fn(async () => undefined);

    await expect(runOnlineRefresh(refresh, () => false)).resolves.toBe(false);
    expect(refresh).not.toHaveBeenCalled();

    await expect(runOnlineRefresh(refresh, () => true)).resolves.toBe(true);
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("keeps cached data successful and paused, then fetches once on reconnect", async () => {
    onlineManager.setOnline(false);
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    client.mount();
    client.setQueryData(["manifest"], { updated_at: "cached" });
    const queryFn = jest.fn(async () => ({ updated_at: "fresh" }));
    const observer = new QueryObserver(client, {
      queryKey: ["manifest"],
      queryFn,
      enabled: false,
    });
    const unsubscribe = observer.subscribe(() => undefined);

    try {
      const pendingRefetch = observer.refetch();
      await Promise.resolve();

      expect(observer.getCurrentResult()).toMatchObject({
        status: "success",
        fetchStatus: "paused",
        isPaused: true,
      });
      expect(queryFn).not.toHaveBeenCalled();

      onlineManager.setOnline(true);
      await pendingRefetch;

      expect(queryFn).toHaveBeenCalledTimes(1);
      expect(observer.getCurrentResult()).toMatchObject({
        data: { updated_at: "fresh" },
        status: "success",
        fetchStatus: "idle",
        isPaused: false,
      });
    } finally {
      unsubscribe();
      client.unmount();
      client.clear();
    }
  });
});
