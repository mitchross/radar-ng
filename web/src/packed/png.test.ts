/**
 * Round trip of a synthetic packed tile: encode an RGB atlas as a PNG (every row filter),
 * decode it the way a PNG decoder does, then read slots back with extractSlot.
 */
import { deflateSync, inflateSync } from "node:zlib";
import { describe, expect, it } from "vitest";
import { extractSlot, slotAddress } from "./format";

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buf: Uint8Array) => {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};

function chunk(type: string, data: Uint8Array): Buffer {
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, "latin1");
  out.set(data, 8);
  out.writeUInt32BE(crc32(out.subarray(4, 8 + data.length)), 8 + data.length);
  return out;
}

const paeth = (a: number, b: number, c: number) => {
  const p = a + b - c;
  const pa = Math.abs(p - a);
  const pb = Math.abs(p - b);
  const pc = Math.abs(p - c);
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
};

/** Predictor for filter `f` at byte i of a row (bpp 3). */
function predict(f: number, row: Uint8Array, prev: Uint8Array, i: number): number {
  const a = i >= 3 ? row[i - 3] : 0;
  const b = prev[i];
  const c = i >= 3 ? prev[i - 3] : 0;
  return [0, a, b, (a + b) >> 1, paeth(a, b, c)][f];
}

function encodeRgbPng(pixels: Uint8Array, width: number, height: number): Buffer {
  const stride = width * 3;
  const raw = new Uint8Array(height * (stride + 1));
  let prev: Uint8Array = new Uint8Array(stride);
  for (let y = 0; y < height; y++) {
    const f = y % 5;
    const row = pixels.subarray(y * stride, (y + 1) * stride);
    raw[y * (stride + 1)] = f;
    for (let i = 0; i < stride; i++) raw[y * (stride + 1) + 1 + i] = (row[i] - predict(f, row, prev, i)) & 0xff;
    prev = row;
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr.set([8, 2, 0, 0, 0], 8); // 8-bit, color type 2 (RGB)
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", new Uint8Array())]);
}

function decodeRgbPng(png: Buffer): { width: number; height: number; pixels: Uint8Array } {
  let pos = 8;
  let width = 0;
  let height = 0;
  const idat: Buffer[] = [];
  while (pos < png.length) {
    const len = png.readUInt32BE(pos);
    const type = png.toString("latin1", pos + 4, pos + 8);
    const data = png.subarray(pos + 8, pos + 8 + len);
    if (type === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      if (data[8] !== 8 || data[9] !== 2) throw new Error("not 8-bit RGB");
    } else if (type === "IDAT") idat.push(data);
    pos += 12 + len;
  }
  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * 3;
  const pixels = new Uint8Array(height * stride);
  let prev: Uint8Array = new Uint8Array(stride);
  for (let y = 0; y < height; y++) {
    const f = raw[y * (stride + 1)];
    const row = pixels.subarray(y * stride, (y + 1) * stride);
    for (let i = 0; i < stride; i++) row[i] = (raw[y * (stride + 1) + 1 + i] + predict(f, row, prev, i)) & 0xff;
    prev = row;
  }
  return { width, height, pixels };
}

describe("packed tile decode", () => {
  const TILE = 512;
  const COLS = 2;
  const SLOTS = 10;
  // A slot's value pattern: distinct per slot, with zeros (no data) on a diagonal band.
  const q = (slot: number, px: number, py: number) => ((px + py) % 97 === 0 ? 0 : 1 + ((slot * 37 + px * 3 + py) % 255));

  it("reads every slot back from a synthetic atlas", () => {
    const blocks = Math.ceil(SLOTS / 3);
    const width = TILE * COLS;
    const height = TILE * Math.ceil(blocks / COLS);
    const pixels = new Uint8Array(width * height * 3);
    for (let s = 0; s < SLOTS; s++) {
      const { channel, x0, y0 } = slotAddress(s, COLS, TILE);
      for (let py = 0; py < TILE; py++) for (let px = 0; px < TILE; px++) pixels[((y0 + py) * width + x0 + px) * 3 + channel] = q(s, px, py);
    }
    const decoded = decodeRgbPng(encodeRgbPng(pixels, width, height));
    expect([decoded.width, decoded.height]).toEqual([1024, 1024]);
    for (let s = 0; s < SLOTS; s++) {
      const got = extractSlot(decoded.pixels, decoded.width, decoded.height, 3, s, COLS, TILE);
      for (const [px, py] of [[0, 0], [1, 0], [511, 0], [0, 511], [200, 311], [511, 511]]) expect(got[py * TILE + px]).toBe(q(s, px, py));
      expect(got.every((v, i) => v === q(s, i % TILE, Math.floor(i / TILE)))).toBe(true);
    }
    // Slots 10 and 11 share block 3 but were never written.
    expect(extractSlot(decoded.pixels, width, height, 3, 10, COLS, TILE).every((v) => v === 0)).toBe(true);
    // A slot past the image's height reads as no data.
    expect(extractSlot(decoded.pixels, width, height, 3, 12, COLS, TILE).every((v) => v === 0)).toBe(true);
  });
});
