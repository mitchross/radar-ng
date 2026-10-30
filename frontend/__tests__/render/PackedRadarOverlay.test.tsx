import { act, render, screen, waitFor } from "@testing-library/react-native";
import { PackedRadarOverlay } from "../../src/components/map/PackedRadarOverlay";
import { useSharedCamera } from "../../src/components/map/WindParticlesOverlay";
import type { PackedFrames } from "../../src/hooks/usePackedFrames";
import { buildLut, parsePackedMeta } from "../../src/lib/packed";
import { useWeatherStore } from "../../src/stores/useWeatherStore";

// Stable shared values and a reaction that runs after each render: the stock mock rebuilds
// shared values every render and never runs reactions.
jest.mock("react-native-worklets", () => ({
  scheduleOnRN: (fn: (...args: unknown[]) => void, ...args: unknown[]) => fn(...args),
}));
jest.mock("react-native-reanimated", () => {
  const React = jest.requireActual<typeof import("react")>("react");
  const make = <T,>(init: T) => {
    let v = init;
    return {
      get value() {
        return v;
      },
      set value(n: T) {
        v = n;
      },
      get: () => v,
      set: (n: T) => {
        v = n;
      },
    };
  };
  return {
    useReducedMotion: () => false,
    useSharedValue: <T,>(init: T) => React.useRef(make(init)).current,
    useDerivedValue: <T,>(fn: () => T) => make(fn()),
    useAnimatedReaction: <T,>(prepare: () => T, react: (next: T, prev: T | null) => void) => {
      const prev = React.useRef<T | null>(null);
      React.useEffect(() => {
        const next = prepare();
        react(next, prev.current);
        prev.current = next;
      });
    },
  };
});

const mockSkia = { drawRect: jest.fn(), uniforms: [] as number[][] };

jest.mock("@shopify/react-native-skia", () => {
  const { View } = jest.requireActual<typeof import("react-native")>("react-native");
  const image = (w: number, h: number) => {
    const img = {
      width: () => w,
      height: () => h,
      makeShaderOptions: () => ({ kind: "image-shader" }),
      makeNonTextureImage: () => img,
    };
    return img;
  };
  const canvas = {
    save: () => 1,
    restore: () => undefined,
    rotate: () => undefined,
    drawRect: (...args: unknown[]) => mockSkia.drawRect(...args),
  };
  return {
    Canvas: ({ onSize, children, ...props }: { onSize?: { set: (s: object) => void }; children?: React.ReactNode }) => {
      onSize?.set({ width: 400, height: 800 });
      return (
        <View testID="packed-radar-canvas" {...props}>
          {children}
        </View>
      );
    },
    Picture: () => null,
    AlphaType: { Unpremul: 3 },
    ColorType: { RGBA_8888: 4 },
    FilterMode: { Nearest: 0 },
    MipmapMode: { None: 0 },
    TileMode: { Clamp: 0 },
    Skia: {
      RuntimeEffect: {
        Make: () => ({
          makeShaderWithChildren: (uniforms: number[]) => {
            mockSkia.uniforms.push(uniforms);
            return {};
          },
        }),
      },
      Image: { MakeImage: () => image(256, 1), MakeImageFromEncoded: () => image(1024, 512) },
      Data: { fromBytes: (b: Uint8Array) => b },
      PictureRecorder: () => ({ beginRecording: () => canvas, finishRecordingAsPicture: () => ({}) }),
      XYWHRect: (x: number, y: number, width: number, height: number) => ({ x, y, width, height }),
      Paint: () => ({ setAntiAlias: () => undefined, setShader: () => undefined }),
    },
  };
});

const META = parsePackedMeta({
  version: 1,
  tile_size: 512,
  min_zoom: 3,
  max_zoom: 6,
  cols: 2,
  max_slots: 24,
  color_key: "reflectivity",
  scale: 0.5,
  offset: -32,
  url_template: "/tiles/packed/{layer}/{bucket}/{z}/{x}/{y}.png?v={v}",
  buckets: [{ id: "B", v: 7, slots: ["t0", "t1", "t2", "t3", "2026-10-15T18:30:38+00:00"] }],
})!;

const packed: PackedFrames = {
  locations: [{ layer: "radar", bucket: "B", v: 7, slot: 4, slots: 5, meta: META, lutKey: "k", channel: 1, x0: 512, y0: 0 }],
  isPacked: [true],
  luts: { k: buildLut({ ranges: [{ min: 5, max: 75, rgba: [0, 200, 0, 200] }], no_data_below: 5 }, 0.5, -32) },
};

// Camera on the center of 512-px tile z5 (8, 11).
const world = 512 * 32;
const LON = ((8.5 * 512) / world) * 360 - 180;
const LAT = (Math.atan(Math.sinh(Math.PI * (1 - (2 * 11.5 * 512) / world))) * 180) / Math.PI;

function Harness() {
  const camera = useSharedCamera(LON, LAT, 5);
  return <PackedRadarOverlay packed={packed} camera={camera} />;
}

const fetchMock = jest.fn(async (_url: string) => ({ status: 200, ok: true, arrayBuffer: async () => new ArrayBuffer(8) }));

beforeEach(() => {
  global.fetch = fetchMock as unknown as typeof fetch;
  fetchMock.mockClear();
  mockSkia.drawRect.mockClear();
  mockSkia.uniforms.length = 0;
  const timestamp = "2026-10-15T18:30:38+00:00";
  useWeatherStore.setState({
    frames: [{ time: Date.parse(timestamp) / 1000, timestamp, path: timestamp, source: "radar" }],
    currentFrameIndex: 0,
    playbackWindow: null,
    radarOpacity: 0.8,
    serverUrl: "https://radar.test",
  });
});

test("mounts a pointer-transparent canvas and draws the slot from the visible tiles", async () => {
  await render(<Harness />);
  const canvas = screen.getByTestId("packed-radar-canvas");
  expect(canvas.props.pointerEvents).toBe("none");

  // The center tile is requested first, at the bucket's version.
  await waitFor(() => expect(fetchMock).toHaveBeenCalled());
  expect(fetchMock.mock.calls[0]?.[0]).toBe("https://radar.test/tiles/packed/radar/B/5/8/11.png?v=7");

  // Later renders draw with the loaded atlases; the mock derived value lags one render.
  await act(async () => useWeatherStore.setState({ radarOpacity: 0.7 }));
  await act(async () => useWeatherStore.setState({ playbackWindow: { start: 0, end: 0 } }));
  await waitFor(() => expect(mockSkia.drawRect).toHaveBeenCalled());
  // Uniforms: tile origin (x, y), scale, block origin (x0, y0), channel, tile size, opacity.
  const center = mockSkia.uniforms.findLast((u) => Math.abs(u[0] + 56) < 1e-6 && Math.abs(u[1] - 144) < 1e-6);
  expect(center?.map((n) => Math.round(n * 1000) / 1000)).toEqual([-56, 144, 1, 512, 0, 1, 512, 0.7]);
});
