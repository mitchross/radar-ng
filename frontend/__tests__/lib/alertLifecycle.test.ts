import { QueryClient, QueryObserver, onlineManager } from "@tanstack/react-query";
import {
  getAlertCollectionSnapshot,
  getAlertEndTime,
  getAlertFreshnessStatus,
  scheduleAlertTransition,
} from "../../src/lib/alertLifecycle";
import { getAlertsScreenState } from "../../src/lib/weatherPresentation";
import type { NWSAlert, NWSAlertCollection } from "../../src/types/weather";

const NOW = Date.parse("2026-09-02T16:00:00Z");

function makeAlert(
  id: string,
  times: Partial<Pick<NWSAlert["properties"], "effective" | "onset" | "expires" | "ends">> = {},
): NWSAlert {
  return {
    id,
    type: "Feature",
    geometry: null,
    properties: {
      id,
      event: "Severe Thunderstorm Warning",
      headline: "A warning",
      description: "Take shelter.",
      instruction: null,
      severity: "Severe",
      urgency: "Immediate",
      effective: "2026-09-02T15:00:00Z",
      onset: "2026-09-02T15:00:00Z",
      expires: "2026-09-02T17:00:00Z",
      ends: null,
      areaDesc: "Test County",
      senderName: "NWS Test",
      ...times,
    },
  };
}

function collection(...features: NWSAlert[]): NWSAlertCollection {
  return { type: "FeatureCollection", features };
}

describe("NWS alert lifecycle", () => {
  afterEach(() => {
    jest.useRealTimers();
    onlineManager.setOnline(true);
  });

  it("shows an effective alert before future onset and uses the earliest valid end", () => {
    const alert = makeAlert("upcoming", {
      onset: "2026-09-02T16:30:00Z",
      expires: "2026-09-02T18:00:00Z",
      ends: "2026-09-02T17:00:00Z",
    });

    const snapshot = getAlertCollectionSnapshot(collection(alert), NOW);

    expect(snapshot.collection.features).toEqual([alert]);
    expect(snapshot.nextTransitionAt).toBe(Date.parse("2026-09-02T17:00:00Z"));
    expect(getAlertEndTime(alert)).toBe(Date.parse("2026-09-02T17:00:00Z"));
  });

  it("falls back to onset only when effective is absent", () => {
    const alert = makeAlert("fallback", { onset: "2026-09-02T16:05:00Z" });
    const withoutEffective = {
      ...alert,
      properties: { ...alert.properties, effective: undefined },
    } as unknown as NWSAlert;

    expect(getAlertCollectionSnapshot(collection(withoutEffective), NOW)).toMatchObject({
      collection: { features: [] },
      nextTransitionAt: Date.parse("2026-09-02T16:05:00Z"),
    });
    expect(getAlertCollectionSnapshot(
      collection(withoutEffective),
      Date.parse("2026-09-02T16:05:00Z"),
    ).collection.features).toEqual([withoutEffective]);
  });

  it("never extends message life past expires when event ends later", () => {
    const alert = makeAlert("message-expiry", {
      expires: "2026-09-02T16:30:00Z",
      ends: "2026-09-02T17:30:00Z",
    });

    expect(getAlertEndTime(alert)).toBe(Date.parse("2026-09-02T16:30:00Z"));
  });

  it("removes an alert at expiration even when the cached response does not change", () => {
    const alert = makeAlert("expiring", { expires: "2026-09-02T16:00:05Z" });
    expect(getAlertCollectionSnapshot(collection(alert), NOW).collection.features).toHaveLength(1);
    expect(
      getAlertCollectionSnapshot(collection(alert), Date.parse("2026-09-02T16:00:05Z"))
        .collection.features,
    ).toHaveLength(0);
  });

  it("wakes the local clock at the next transition instead of waiting for a poll", () => {
    jest.useFakeTimers();
    jest.setSystemTime(NOW);
    const alert = makeAlert("timer", { expires: "2026-09-02T16:00:05Z" });
    let visible = getAlertCollectionSnapshot(collection(alert), Date.now()).collection.features;
    const initial = getAlertCollectionSnapshot(collection(alert), Date.now());
    const cleanup = scheduleAlertTransition(initial.nextTransitionAt, () => {
      visible = getAlertCollectionSnapshot(collection(alert), Date.now()).collection.features;
    });

    jest.advanceTimersByTime(4_999);
    expect(visible).toHaveLength(1);
    jest.advanceTimersByTime(1);
    expect(visible).toHaveLength(0);
    cleanup();
  });

  it.each([
    ["effective", { effective: "today" }],
    ["expires", { expires: "2026-02-30T17:00:00Z" }],
    ["missing expires", { expires: undefined }],
    ["ends", { ends: "2026-09-02" }],
    ["fallback onset", { effective: undefined, onset: "soon" }],
  ] as const)("fails closed for malformed %s timestamps", (_label, times) => {
    const malformed = {
      ...makeAlert("bad"),
      properties: { ...makeAlert("bad").properties, ...times },
    } as unknown as NWSAlert;
    const snapshot = getAlertCollectionSnapshot(collection(malformed), NOW);

    expect(snapshot.collection.features).toEqual([]);
    expect(snapshot.invalidCount).toBe(1);
    expect(snapshot.nextTransitionAt).toBeNull();
  });

  it("preserves a still-valid cached alert offline and labels it plainly", () => {
    const snapshot = getAlertCollectionSnapshot(collection(makeAlert("cached")), NOW);
    const status = getAlertFreshnessStatus({
      hasCachedData: true,
      activeCount: snapshot.collection.features.length,
      invalidCount: snapshot.invalidCount,
      isOnline: false,
      refreshFailed: false,
      isPending: false,
    });

    expect(snapshot.collection.features.map((alert) => alert.id)).toEqual(["cached"]);
    expect(status).toMatchObject({ kind: "offline", label: "OFFLINE" });
    expect(status.accessibilityLabel).toContain("Showing still-active alerts");
  });

  it("keeps valid cached content after refresh failure but never claims cached emptiness is all-clear", () => {
    const cached = getAlertCollectionSnapshot(collection(makeAlert("cached")), NOW).collection;
    const expiredCache = getAlertCollectionSnapshot(
      collection(makeAlert("expired", { expires: "2026-09-02T16:00:00Z" })),
      NOW,
    ).collection;
    const stale = getAlertFreshnessStatus({
      hasCachedData: true,
      activeCount: 1,
      invalidCount: 0,
      isOnline: true,
      refreshFailed: true,
      isPending: false,
    });
    const failedEmpty = getAlertFreshnessStatus({
      hasCachedData: true,
      activeCount: 0,
      invalidCount: 0,
      isOnline: true,
      refreshFailed: true,
      isPending: false,
    });

    expect(getAlertsScreenState({
      data: cached,
      isLoading: false,
      isPending: false,
      freshness: stale.kind,
    })).toEqual({ kind: "content" });
    expect(getAlertsScreenState({
      data: expiredCache,
      isLoading: false,
      isPending: false,
      freshness: failedEmpty.kind,
    })).toEqual({ kind: "error" });
    expect(stale).toMatchObject({ kind: "stale", label: "STALE" });
  });

  it("retains the cached response when a real Query refresh fails", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const key = ["alerts", 42.9, -85.6] as const;
    client.setQueryData(key, collection(makeAlert("cached")));
    const observer = new QueryObserver(client, {
      queryKey: key,
      queryFn: async () => {
        throw new Error("NWS unavailable");
      },
      enabled: false,
    });
    const unsubscribe = observer.subscribe(() => undefined);

    try {
      await observer.refetch();
      const failed = observer.getCurrentResult();
      const snapshot = getAlertCollectionSnapshot(failed.data!, NOW);

      expect(failed.isError).toBe(true);
      expect(snapshot.collection.features.map((alert) => alert.id)).toEqual(["cached"]);
      expect(getAlertFreshnessStatus({
        hasCachedData: true,
        activeCount: snapshot.collection.features.length,
        invalidCount: snapshot.invalidCount,
        isOnline: true,
        refreshFailed: failed.isError,
        isPending: failed.isPending,
      }).kind).toBe("stale");
    } finally {
      unsubscribe();
      client.clear();
    }
  });

  it("keeps cached alerts through an offline request and replaces them on reconnect", async () => {
    onlineManager.setOnline(false);
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    client.mount();
    const key = ["alerts", 42.9, -85.6] as const;
    client.setQueryData(key, collection(makeAlert("cached")));
    const queryFn = jest.fn(async () => collection(makeAlert("fresh")));
    const observer = new QueryObserver(client, { queryKey: key, queryFn, enabled: false });
    const unsubscribe = observer.subscribe(() => undefined);

    try {
      const reconnectResult = observer.refetch();
      await Promise.resolve();
      const paused = observer.getCurrentResult();
      const cachedSnapshot = getAlertCollectionSnapshot(paused.data!, NOW);

      expect(paused.fetchStatus).toBe("paused");
      expect(cachedSnapshot.collection.features.map((alert) => alert.id)).toEqual(["cached"]);
      expect(queryFn).not.toHaveBeenCalled();

      onlineManager.setOnline(true);
      await reconnectResult;
      const reconnected = observer.getCurrentResult();

      expect(queryFn).toHaveBeenCalledTimes(1);
      expect(getAlertCollectionSnapshot(reconnected.data!, NOW).collection.features.map(
        (alert) => alert.id,
      )).toEqual(["fresh"]);
      expect(getAlertFreshnessStatus({
        hasCachedData: true,
        activeCount: 1,
        invalidCount: 0,
        isOnline: true,
        refreshFailed: reconnected.isError,
        isPending: reconnected.isPending,
      }).kind).toBe("current");
    } finally {
      unsubscribe();
      client.unmount();
      client.clear();
    }
  });
});
