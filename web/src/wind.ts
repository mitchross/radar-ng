/**
 * Animated wind particles over the map (HRRR 10 m wind). Particles live in
 * screen space and sample the field through map.unproject, so pans and zooms
 * need no re-projection; trails fade by redrawing the canvas slightly transparent.
 */
import type { Map as MLMap } from "maplibre-gl";

import { sampleWind, type WindField } from "./shared/windSample";

export { sampleWind, type WindField };

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
