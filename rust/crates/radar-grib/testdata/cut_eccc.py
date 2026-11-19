"""Cut a real ECCC RDPS rotated-grid GRIB2 (template 3.1, packing 5.40) to a small window.

Usage: cut.py IN OUT R0 R1 C0 C1   (rows/cols inclusive, row 0 = first scanned row)
Decodes section 7 with the pure-Rust openjp2 CLI, crops, re-encodes losslessly, rewrites
sections 3 and 5 for the new size and corners. Every other byte is copied.
"""
import struct, subprocess, sys, zlib, os

CLI = "/opt/data/cache/scratch/eccc/cli/bin/openjp2"


def sm_get(b, off):
    v = struct.unpack(">I", b[off:off + 4])[0]
    return -(v & 0x7FFFFFFF) if v & 0x80000000 else v


def sm_put(v):
    return struct.pack(">I", (0x80000000 | -v) if v < 0 else v)


def read_png16(path):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    pos, idat, ihdr = 8, b"", None
    while pos < len(data):
        n = struct.unpack(">I", data[pos:pos + 4])[0]
        t = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + n]
        if t == b"IHDR":
            ihdr = struct.unpack(">IIBBBBB", body)
        elif t == b"IDAT":
            idat += body
        pos += 12 + n
    w, h, depth, ctype = ihdr[0], ihdr[1], ihdr[2], ihdr[3]
    assert ctype == 0, f"colour type {ctype}"
    bpp = depth // 8
    raw = zlib.decompress(idat)
    stride = w * bpp
    out, prev, p = [], bytearray(stride), 0
    for _ in range(h):
        f = raw[p]
        line = bytearray(raw[p + 1:p + 1 + stride])
        p += 1 + stride
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            if f == 1:
                line[i] = (line[i] + a) & 255
            elif f == 2:
                line[i] = (line[i] + b) & 255
            elif f == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif f == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 255
        prev = line
        if bpp == 2:
            out.append([struct.unpack(">H", line[2 * k:2 * k + 2])[0] for k in range(w)])
        else:
            out.append(list(line))
    return w, h, depth, out


def write_png16(path, rows):
    h, w = len(rows), len(rows[0])
    raw = b"".join(b"\x00" + b"".join(struct.pack(">H", v) for v in r) for r in rows)
    def chunk(t, body):
        return struct.pack(">I", len(body)) + t + body + struct.pack(">I", zlib.crc32(t + body) & 0xFFFFFFFF)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 16, 0, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    open(path, "wb").write(png)


def main():
    src, dst = sys.argv[1], sys.argv[2]
    r0, r1, c0, c1 = map(int, sys.argv[3:7])
    b = open(src, "rb").read()
    assert b[:4] == b"GRIB" and b[7] == 2
    pos, secs = 16, {}
    order = []
    while b[pos:pos + 4] != b"7777":
        n = struct.unpack(">I", b[pos:pos + 4])[0]
        num = b[pos + 4]
        secs[num] = bytearray(b[pos:pos + n])
        order.append(num)
        pos += n
    assert order == [1, 3, 4, 5, 6, 7], order
    s3, s5, s7 = secs[3], secs[5], secs[7]
    assert struct.unpack(">H", s3[12:14])[0] == 1 and struct.unpack(">H", s5[9:11])[0] == 40
    ni, nj = struct.unpack(">II", s3[30:38])
    la1, lo1 = sm_get(s3, 46), sm_get(s3, 50)
    di, dj = struct.unpack(">II", s3[63:71])
    scan = s3[71]
    assert scan == 0x40, f"scan {scan:#x}"
    tmp = dst + ".tmp"
    j2k = tmp + ".j2k"
    open(j2k, "wb").write(bytes(s7[5:]))
    subprocess.run([CLI, "decompress", "--area", f"{c0},{r0},{c1 + 1},{r1 + 1}", j2k, tmp + ".png"], check=True,
                   stdout=subprocess.DEVNULL)
    w, h, depth, rows = read_png16(tmp + ".png")
    assert (w, h) == (c1 - c0 + 1, r1 - r0 + 1), (w, h)
    nbits = s5[19]
    if depth == 16 and nbits < 16:
        # The CLI widens n-bit samples to 16 bits; undo it so the stored values are the packed ones.
        top = (1 << nbits) - 1
        rows = [[round(v * top / 65535) for v in r] for r in rows]
    print("png depth", depth, "nbits", nbits, "max", max(max(r) for r in rows))
    write_png16(tmp + ".crop.png", rows)
    subprocess.run([CLI, "compress", tmp + ".crop.png", tmp + ".crop.j2k"], check=True, stdout=subprocess.DEVNULL)
    cs = open(tmp + ".crop.j2k", "rb").read()
    s3[30:38] = struct.pack(">II", w, h)
    s3[6:10] = struct.pack(">I", w * h)
    la2, lo2 = sm_get(s3, 55), sm_get(s3, 59)
    # Steps from the corners, as decoders do; Di/Dj are rounded to micro-degrees.
    sj, si = (la2 - la1) / (nj - 1), ((lo2 - lo1) % 360000000) / (ni - 1)
    s3[46:50] = sm_put(la1 + round(r0 * sj))
    s3[50:54] = sm_put(lo1 + round(c0 * si))
    s3[55:59] = sm_put(la1 + round(r1 * sj))
    s3[59:63] = sm_put(lo1 + round(c1 * si))
    s5[5:9] = struct.pack(">I", w * h)
    s7 = struct.pack(">I", 5 + len(cs)) + b"\x07" + cs
    body = b"".join(bytes(secs[k]) if k != 7 else s7 for k in order)
    total = 16 + len(body) + 4
    out = b"GRIB" + b[4:8] + struct.pack(">Q", total) + body + b"7777"
    open(dst, "wb").write(out)
    for f in (j2k, tmp + ".png", tmp + ".crop.png", tmp + ".crop.j2k"):
        os.remove(f)
    print(dst, len(out), "bytes", w, "x", h, "la1", la1 + round(r0 * sj), "lo1", lo1 + round(c0 * si), "di", di, "dj", dj)


main()
