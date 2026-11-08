import { useState } from "react";
import { act, fireEvent, screen, waitFor } from "@testing-library/react-native";
import { SpcOverlay } from "../../src/components/map/SpcOverlay";
import { SpcSheet } from "../../src/components/map/SpcSheet";
import { WarningsOverlay, type WarningDetails } from "../../src/components/map/WarningsOverlay";
import { StormCellsOverlay } from "../../src/components/map/StormCellsOverlay";
import { TropicalOverlay, type TropicalStormDetails } from "../../src/components/map/TropicalOverlay";
import { StormCellSheet, WarningSheet } from "../../src/components/map/FeatureSheets";
import { TropicalDetailSheet } from "../../src/components/map/TropicalDetailSheet";
import type { StormCellInfo } from "../../src/lib/stormTracks";
import type { PinnedPoint } from "../../src/components/inspector/Eyedropper";
import { InspectorPanel } from "../../src/components/inspector/Eyedropper";
import { useWeatherStore } from "../../src/stores/useWeatherStore";
import { fetchSpc, fetchOutlook } from "../../src/lib/api";
import { renderWithProviders } from "./helpers";
import type { SpcProduct } from "../../src/lib/spc";

jest.mock("@maplibre/maplibre-react-native", () => ({
  GeoJSONSource: (props: Record<string, unknown>) => {
    const React = require("react");
    const { View } = require("react-native");
    return React.createElement(View, { ...props, testID: props.id });
  },
  Layer: () => null,
  Marker: () => null,
}));
jest.mock("../../src/hooks/useBasemapStyle", () => ({ useBasemapStyle: () => ({ labelFont: ["Noto Sans Regular"] }) }));
jest.mock("../../src/hooks/useAlerts", () => ({ useAlerts: () => ({ data: null }) }));
jest.mock("../../src/hooks/useStormCells", () => ({ useStormCells: () => ({ data: { features: [
  { geometry: { type: "Point", coordinates: [50, 50] }, properties: { cell_id: 7, peak_dbz: 58 } },
] } }) }));
jest.mock("../../src/hooks/useTropical", () => ({ useTropical: () => ({ data: { type: "FeatureCollection", features: [] } }) }));
jest.mock("../../src/hooks/useAlertExplanation", () => ({ useAlertExplanation: () => ({ explanation: null, loading: false }) }));
jest.mock("../../src/lib/api", () => ({ fetchSpc: jest.fn(), fetchOutlook: jest.fn(), fetchMapAlerts: jest.fn(async () => ({ type: "FeatureCollection", features: [] })) }));
const spc = jest.mocked(fetchSpc);
const outlook = jest.mocked(fetchOutlook);
const product: SpcProduct = { id: "2026-ww-0001", number: 1, type: "tornado", issued: "2026-11-08T00:00:00Z", expires: "2099-01-01T00:00:00Z",
  text: "Complete public watch text", geometry_note: "SPC aviation watch approximation", probabilities: [{ label: "Strong tornadoes", value: "<5%" }] };
const geometry: GeoJSON.Polygon = { type: "Polygon", coordinates: [[[0, 0], [10, 0], [10, 3], [0, 3], [0, 0]]] };

beforeEach(() => {
  jest.clearAllMocks();
  useWeatherStore.setState({ serverUrl: "https://radar.test", extrasVisible: false, spcWatchesVisible: false, spcDiscussionsVisible: false });
  spc.mockResolvedValue({ type: "FeatureCollection", features: [
    { type: "Feature", geometry, properties: product },
    { type: "Feature", geometry, properties: { ...product, expires: "2000-01-01T00:00:00Z" } },
  ] });
  outlook.mockImplementation(async (_server, day) => {
    if (day === 2) throw new Error("upstream unavailable");
    return { type: "FeatureCollection", features: [{ type: "Feature", geometry,
      properties: { label: "SLGT", name: "Slight risk", expire: day === 3 ? "2000-01-01T00:00:00Z" : "2099-01-01T00:00:00Z" } }] };
  });
});

test("SPC toggles work independently of storm extras and taps select full active products", async () => {
  const selected = jest.fn();
  await renderWithProviders(<><SpcOverlay kind="watches" onSelect={selected} /><SpcOverlay kind="discussions" onSelect={selected} /></>);
  expect(spc).not.toHaveBeenCalled();
  await act(async () => useWeatherStore.getState().setSpcVisible("watches", true));
  await waitFor(() => expect(screen.getByTestId("spc-watches").props.data.features).toHaveLength(1));
  expect(screen.getByTestId("spc-discussions").props.data.features).toEqual([]);
  expect(spc).toHaveBeenCalledWith("https://radar.test", "watches", expect.anything());
  const stopPropagation = jest.fn();
  await fireEvent(screen.getByTestId("spc-watches"), "press", { nativeEvent: { features: [{ properties: product }], lngLat: [1, 2] }, stopPropagation });
  expect(selected).toHaveBeenCalledWith({ product, lon: 1, lat: 2 });
  expect(stopPropagation).toHaveBeenCalledTimes(1);
  selected.mockClear();
  await fireEvent(screen.getByTestId("spc-watches"), "press", { nativeEvent: { features: [{ properties: { ...product, expires: "2000-01-01T00:00:00Z" } }], lngLat: [1, 2] }, stopPropagation });
  expect(selected).not.toHaveBeenCalled();
  await act(async () => useWeatherStore.getState().setSpcVisible("watches", false));
  expect(screen.getByTestId("spc-watches").props.data.features).toEqual([]);
});

test("SPC sheet shows full text, probability rows, source and Day 1-3 outlook availability", async () => {
  await renderWithProviders(<SpcSheet selection={{ product, lat: 1, lon: 1 }} onClose={jest.fn()} />);
  expect(screen.getByText(product.text)).toBeOnTheScreen();
  expect(screen.getByText("Strong tornadoes")).toBeOnTheScreen();
  expect(screen.getByText("<5%")).toBeOnTheScreen();
  expect(screen.getByText("NOAA/NWS Storm Prediction Center")).toBeOnTheScreen();
  expect(screen.getByText(product.geometry_note)).toBeOnTheScreen();
  await waitFor(() => expect(screen.getByText("Day 1: SLGT · Slight risk")).toBeOnTheScreen());
  expect(screen.getByText("Day 2: Outlook unavailable")).toBeOnTheScreen();
  expect(screen.getByText("Day 3: Outlook expired")).toBeOnTheScreen();
  expect(outlook.mock.calls.map((c) => c[1]).sort()).toEqual([1, 2, 3]);
});

test("point outlooks append to the existing layer inspector", async () => {
  await renderWithProviders(<InspectorPanel pinned={{ lat: 1, lon: 1 }} inspect={{ layerLabel: "REFLECTIVITY", readout: "35 dBZ", sourceLabel: "Grid" }} onClear={jest.fn()} />);
  expect(screen.getByText("35 dBZ")).toBeOnTheScreen();
  expect(screen.getByText("Grid")).toBeOnTheScreen();
  expect(screen.getByRole("button", { name: "Clear inspected point" })).toBeOnTheScreen();
  await waitFor(() => expect(screen.getByText("Day 1: SLGT · Slight risk")).toBeOnTheScreen());
});

function InterceptedTap({ kind }: { kind: "warning" | "cell" | "tropical" }) {
  const [point, setPoint] = useState<PinnedPoint>();
  const [warning, setWarning] = useState<WarningDetails | null>(null);
  const [cell, setCell] = useState<StormCellInfo | null>(null);
  const [storm, setStorm] = useState<TropicalStormDetails | null>(null);
  return <>
    {kind === "warning" ? <WarningsOverlay onSelect={(value, tap) => { setWarning(value); setPoint(tap); }} /> : null}
    {kind === "cell" ? <StormCellsOverlay onSelect={(value, tap) => { setCell(value); setPoint(tap); }} /> : null}
    {kind === "tropical" ? <TropicalOverlay onSelect={(value, tap) => { setStorm(value); setPoint(tap); }} /> : null}
    <WarningSheet warning={warning} point={point} onClose={jest.fn()} />
    <StormCellSheet cell={cell} point={point} onClose={jest.fn()} />
    <TropicalDetailSheet storm={storm} point={point} onClose={jest.fn()} />
  </>;
}

test.each([
  ["warning", "warnings-src", { id: "warn-1", event: "Tornado Warning" }],
  ["cell", "storms-src", { cell_id: 7 }],
  ["tropical", "tropical-src", { kind: "position", storm_id: "al012026", name: "Test storm" }],
] as const)("%s intercepted taps show outlooks at the tap, not the feature position", async (kind, source, properties) => {
  useWeatherStore.setState({ extrasVisible: true });
  await renderWithProviders(<InterceptedTap kind={kind} />);
  expect(screen.queryByLabelText("SPC outlook at inspected point")).toBeNull();
  const stopPropagation = jest.fn();
  const press = (lngLat: number[]) => fireEvent(screen.getByTestId(source), "press", {
    nativeEvent: { features: [{ properties, geometry: { type: "Point", coordinates: [50, 50] } }], lngLat }, stopPropagation,
  });
  await press([5, 2]);
  await waitFor(() => expect(screen.getByText("Day 1: SLGT · Slight risk")).toBeOnTheScreen());
  expect(screen.getByText("Day 2: Outlook unavailable")).toBeOnTheScreen();
  expect(screen.getByText("Day 3: Outlook expired")).toBeOnTheScreen();
  await press([50, 50]);
  await waitFor(() => expect(screen.getByText("Day 1: No outlined risk")).toBeOnTheScreen());
  expect(stopPropagation).toHaveBeenCalledTimes(2);
});

test("tropical taps without a valid storm propagate to the point inspector", async () => {
  const selected = jest.fn();
  await renderWithProviders(<TropicalOverlay onSelect={selected} />);
  const stopPropagation = jest.fn();
  for (const properties of [{ kind: "cone" }, { kind: "position" }]) {
    await fireEvent(screen.getByTestId("tropical-src"), "press", {
      nativeEvent: { features: [{ properties }], lngLat: [1, 2] }, stopPropagation,
    });
  }
  expect(selected).not.toHaveBeenCalled();
  expect(stopPropagation).not.toHaveBeenCalled();
});

test("existing overlay callbacks still receive one argument when a tap has no coordinates", async () => {
  const warning = jest.fn();
  const cell = jest.fn();
  const storm = jest.fn();
  useWeatherStore.setState({ extrasVisible: true });
  await renderWithProviders(<>
    <WarningsOverlay onSelect={warning} />
    <StormCellsOverlay onSelect={cell} />
    <TropicalOverlay onSelect={storm} />
  </>);
  for (const [source, properties, selected] of [
    ["warnings-src", { id: "warn-1", event: "Tornado Warning" }, warning],
    ["storms-src", { cell_id: 7 }, cell],
    ["tropical-src", { kind: "position", storm_id: "al012026", name: "Test storm" }, storm],
  ] as const) {
    await fireEvent(screen.getByTestId(source), "press", { nativeEvent: { features: [{ properties }] }, stopPropagation: jest.fn() });
    expect(selected).toHaveBeenCalledTimes(1);
    expect(selected.mock.calls[0]).toHaveLength(1);
  }
});
