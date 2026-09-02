"""Rewrite the PLTE/tRNS chunks of an indexed-colour (mode "P") PNG.

A tile pyramid is rendered once as class indices; every other palette is the
same IDAT with a different 256-entry lookup table. Swapping the two colour
chunks (and their CRCs) is byte-cheap compared with re-encoding the image.
"""

from __future__ import annotations

import struct
import zlib
from collections.abc import Iterator

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
_INDEXED_COLOR_TYPE = 3


def iter_chunks(png: bytes) -> Iterator[tuple[bytes, bytes]]:
    """Yield (type, payload) for every chunk, validating structure (not CRCs)."""
    if not png.startswith(PNG_SIGNATURE):
        raise ValueError("not a PNG: bad signature")
    pos = len(PNG_SIGNATURE)
    end = len(png)
    while pos < end:
        if pos + 8 > end:
            raise ValueError("truncated PNG chunk header")
        (length,) = struct.unpack(">I", png[pos : pos + 4])
        ctype = png[pos + 4 : pos + 8]
        payload_start = pos + 8
        payload_end = payload_start + length
        if payload_end + 4 > end:
            raise ValueError(f"truncated PNG chunk {ctype!r}")
        yield ctype, png[payload_start:payload_end]
        pos = payload_end + 4


def make_chunk(ctype: bytes, payload: bytes) -> bytes:
    crc = zlib.crc32(ctype + payload) & 0xFFFFFFFF
    return struct.pack(">I", len(payload)) + ctype + payload + struct.pack(">I", crc)


def replace_palette(png: bytes, rgb: bytes, alpha: bytes | None) -> bytes:
    """Return `png` with its PLTE (and tRNS) replaced; IDAT and all other chunks untouched.

    `rgb` is 3 bytes per entry (1..256 entries); `alpha` is one byte per entry
    and may be shorter than the palette (PNG treats missing entries as opaque).
    `alpha=None` removes any existing tRNS chunk.
    """
    if len(rgb) % 3 or not 1 <= len(rgb) // 3 <= 256:
        raise ValueError("rgb must be 3 bytes per entry, 1..256 entries")
    entries = len(rgb) // 3
    if alpha is not None and len(alpha) > entries:
        raise ValueError("alpha has more entries than the palette")

    out = bytearray(PNG_SIGNATURE)
    saw_plte = False
    for ctype, payload in iter_chunks(png):
        if ctype == b"IHDR":
            color_type = payload[9]
            if color_type != _INDEXED_COLOR_TYPE:
                raise ValueError(f"PNG colour type {color_type} is not indexed (3)")
            out += make_chunk(ctype, payload)
        elif ctype == b"PLTE":
            saw_plte = True
            out += make_chunk(b"PLTE", rgb)
            if alpha is not None:
                out += make_chunk(b"tRNS", alpha)
        elif ctype == b"tRNS":
            # Emitted right after PLTE above (spec ordering), so drop the original.
            if not saw_plte:
                raise ValueError("tRNS before PLTE")
            continue
        elif ctype == b"IDAT" and not saw_plte:
            raise ValueError("indexed PNG has no PLTE chunk")
        else:
            out += make_chunk(ctype, payload)
    if not saw_plte:
        raise ValueError("indexed PNG has no PLTE chunk")
    return bytes(out)
