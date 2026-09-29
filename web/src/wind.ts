/**
 * Animated wind particles over the map (HRRR 10 m wind). Particles live in
 * screen space and sample the field through map.unproject, so pans and zooms
 * need no re-projection; trails fade by redrawing the canvas slightly transparent.
 */
import type { Map as MLMap } from "maplibre-gl";

export interface WindField {
  ok: true;
  timestamp: string;
  width: number;
  height: number;
  lat_min: number;
  lat_max: number;
  lon_min: number;
  lon_max: number;
  u_min: number;
  u_max: number;
  v_min: number;
  v_max: number;
  u: number[];
  v: number[];
}

const FILL = -128;

/** (u, v) in mph at a point, or null outside the field / model domain. */
export function sampleWind(f: WindField, lat: number, lon: number): [number, number] | null {
  const fx = ((lon - f.lon_min) / (f.lon_max - f.lon_min)) * (f.width - 1);
  const fy = ((f.lat_max - lat) / (f.lat_max - f.lat_min)) * (f.height - 1);
  if (!(fx >= 0 && fx <= f.width - 1 && fy >= 0 && fy <= f.height - 1)) return null;
  const x0 = Math.floor(fx);
  const y0 = Math.floor(fy);
  const x1 = Math.min(f.width - 1, x0 + 1);
  const y1 = Math.min(f.height - 1, y0 + 1);
  const idx = [y0 * f.width + x0, y0 * f.width + x1, y1 * f.width + x0, y1 * f.width + x1];
  if (idx.some((i) => f.u[i] === FILL || f.v[i] === FILL)) return null;
  const dx = fx - x0;
  const dy = fy - y0;
  const w = [(1 - dx) * (1 - dy), dx * (1 - dy), (1 - dx) * dy, dx * dy];
  const un = (n: number) => f.u_min + ((n + 127) / 254) * (f.u_max - f.u_min);
  const vn = (n: number) => f.v_min + ((n + 127) / 254) * (f.v_max - f.v_min);
  let u = 0;
  let v = 0;
  idx.forEach((i, k) => {
    u += un(f.u[i]) * w[k];
    v += vn(f.v[i]) * w[k];
  });
  return [u, v];
}

const SPEED_COLORS: [number, string][] = [
  [0, "rgba(160,200,255,0.55)"],
  [8, "rgba(120,230,255,0.75)"],
  [15, "rgba(140,255,160,0.85)"],
  [25, "rgba(255,230,90,0.9)"],
  [35, "rgba(255,150,60,0.95)"],
  [50, "rgba(255,70,120,1)"],
];
const colorFor = (mph: number) => {
  let c = SPEED_COLORS[0][1];
  for (const [min, col] of SPEED_COLORS) if (mph >= min) c = col;
  return c;
};

interface Particle {
  x: number;
  y: number;
  age: number;
}

export class WindLayer {
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private particles: Particle[] = [];
  private field: WindField | null = null;
  private fieldKey = "";
  private raf = 0;
  private moving = false;
  enabled = false;

  constructor(private map: MLMap) {
    this.canvas = document.createElement("canvas");
    this.canvas.className = "wind-canvas";
    map.getContainer().appendChild(this.canvas);
    this.ctx = this.canvas.getContext("2d")!;
    const clear = () => {
      this.moving = true;
      this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
    };
    map.on("movestart", clear);
    map.on("moveend", () => {
      this.moving = false;
      this.seed();
    });
    map.on("resize", () => this.resize());
    this.resize();
  }

  private resize() {
    const r = window.devicePixelRatio || 1;
    const { clientWidth: w, clientHeight: h } = this.map.getContainer();
    this.canvas.width = w * r;
    this.canvas.height = h * r;
    this.canvas.style.width = `${w}px`;
    this.canvas.style.height = `${h}px`;
    this.ctx.setTransform(r, 0, 0, r, 0, 0);
    this.seed();
  }

  private seed() {
    const { clientWidth: w, clientHeight: h } = this.map.getContainer();
    const n = Math.round(Math.min(4000, (w * h) / 520));
    this.particles = Array.from({ length: n }, () => ({ x: Math.random() * w, y: Math.random() * h, age: Math.floor(Math.random() * 80) }));
  }

  /** Load the HRRR wind hour nearest `timestamp`; returns false when the server has no wind grids. */
  async load(timestamp: string | null): Promise<boolean> {
    const key = timestamp ? timestamp.slice(0, 13) : "latest";
    if (key === this.fieldKey && this.field) return true;
    try {
      const res = await fetch(`/api/wind-field/${encodeURIComponent(timestamp ?? "latest")}`);
      const body = (await res.json()) as WindField | { ok: false };
      if (!res.ok || !body.ok) return false;
      this.field = body;
      this.fieldKey = key;
      return true;
    } catch {
      return false;
    }
  }

  setEnabled(on: boolean) {
    this.enabled = on;
    this.canvas.hidden = !on;
    cancelAnimationFrame(this.raf);
    if (on) this.raf = requestAnimationFrame(() => this.frame());
    else this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
  }

  private frame() {
    if (!this.enabled) return;
    this.raf = requestAnimationFrame(() => this.frame());
    const f = this.field;
    if (!f || this.moving) return;
    const { clientWidth: w, clientHeight: h } = this.map.getContainer();
    const ctx = this.ctx;
    ctx.globalCompositeOperation = "destination-in";
    ctx.fillStyle = "rgba(0,0,0,0.92)";
    ctx.fillRect(0, 0, w, h);
    ctx.globalCompositeOperation = "source-over";
    ctx.lineWidth = 1.3;
    // A 20 mph wind moves ~1.3 px per frame regardless of zoom, like a screen-space flow.
    const scale = 0.065;
    const buckets = new Map<string, number[]>();
    for (const p of this.particles) {
      const ll = this.map.unproject([p.x, p.y]);
      const wind = sampleWind(f, ll.lat, ll.lng);
      if (!wind || ++p.age > 90) {
        p.x = Math.random() * w;
        p.y = Math.random() * h;
        p.age = 0;
        continue;
      }
      const [u, v] = wind;
      const nx = p.x + u * scale;
      const ny = p.y - v * scale;
      const color = colorFor(Math.hypot(u, v));
      let seg = buckets.get(color);
      if (!seg) buckets.set(color, (seg = []));
      seg.push(p.x, p.y, nx, ny);
      p.x = nx;
      p.y = ny;
      if (nx < 0 || ny < 0 || nx > w || ny > h) p.age = 999;
    }
    for (const [color, seg] of buckets) {
      ctx.strokeStyle = color;
      ctx.beginPath();
      for (let i = 0; i < seg.length; i += 4) {
        ctx.moveTo(seg[i], seg[i + 1]);
        ctx.lineTo(seg[i + 2], seg[i + 3]);
      }
      ctx.stroke();
    }
  }
}
