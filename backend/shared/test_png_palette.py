"""PLTE/tRNS rewrite must be pixel-identical to encoding the palette directly."""

import io

import numpy as np
import pytest
from PIL import Image

from backend.shared.png_palette import iter_chunks, make_chunk, replace_palette
from backend.shared.tiler import encode_indexed_png


def _lut(seed: int, n: int) -> np.ndarray:
    rng = np.random.default_rng(seed)
    lut = rng.integers(0, 256, size=(n, 4), dtype=np.uint8)
    lut[0] = 0
    return lut


def _decode(png: bytes) -> np.ndarray:
    return np.asarray(Image.open(io.BytesIO(png)).convert("RGBA"))


@pytest.mark.parametrize("n_classes", [2, 12, 16, 17, 200])
def test_replace_palette_matches_direct_encode(n_classes):
    rng = np.random.default_rng(n_classes)
    tile = rng.integers(0, n_classes, size=(256, 256), dtype=np.uint8)
    a, b = _lut(1, n_classes), _lut(2, n_classes)

    direct_a = encode_indexed_png(tile, a[:, :3].tobytes(), a[:, 3].tobytes())
    direct_b = encode_indexed_png(tile, b[:, :3].tobytes(), b[:, 3].tobytes())
    rewritten_b = replace_palette(direct_a, b[:, :3].tobytes(), b[:, 3].tobytes())

    assert np.array_equal(_decode(rewritten_b), b[tile])
    assert np.array_equal(_decode(rewritten_b), _decode(direct_b))
    # Only the colour chunks change; IDAT is shared byte-for-byte.
    chunks_a = dict(iter_chunks(direct_a))
    chunks_b = dict(iter_chunks(rewritten_b))
    assert chunks_a[b"IDAT"] == chunks_b[b"IDAT"]
    assert chunks_a[b"IHDR"] == chunks_b[b"IHDR"]
    assert chunks_b[b"PLTE"] == b[:, :3].tobytes()
    assert chunks_b[b"tRNS"] == b[:, 3].tobytes()


def test_replace_palette_recomputes_crc():
    import zlib

    tile = np.zeros((8, 8), dtype=np.uint8)
    tile[2:6, 2:6] = 1
    png = encode_indexed_png(tile, bytes([0, 0, 0, 255, 0, 0]), bytes([0, 255]))
    out = replace_palette(png, bytes([0, 0, 0, 0, 0, 255]), bytes([0, 128]))
    pos = 8
    while pos < len(out):
        length = int.from_bytes(out[pos : pos + 4], "big")
        ctype = out[pos + 4 : pos + 8]
        payload = out[pos + 8 : pos + 8 + length]
        crc = int.from_bytes(out[pos + 8 + length : pos + 12 + length], "big")
        assert crc == zlib.crc32(ctype + payload) & 0xFFFFFFFF, ctype
        pos += 12 + length


def test_replace_palette_inserts_trns_when_missing_and_drops_when_none():
    tile = np.array([[0, 1], [1, 0]], dtype=np.uint8)
    img = Image.frombuffer("P", (2, 2), tile.tobytes(), "raw", "P", 0, 1)
    img.putpalette(bytes([0, 0, 0, 255, 255, 255]))
    buf = io.BytesIO()
    img.save(buf, "PNG", optimize=False)
    opaque = buf.getvalue()
    assert b"tRNS" not in dict(iter_chunks(opaque))

    with_alpha = replace_palette(opaque, bytes([9, 9, 9, 1, 2, 3]), bytes([0, 255]))
    decoded = _decode(with_alpha)
    assert decoded[0, 0, 3] == 0 and tuple(decoded[0, 1]) == (1, 2, 3, 255)
    stripped = replace_palette(with_alpha, bytes([9, 9, 9, 1, 2, 3]), None)
    assert b"tRNS" not in dict(iter_chunks(stripped))
    assert _decode(stripped)[0, 0, 3] == 255


def test_replace_palette_rejects_non_indexed_png():
    rgba = Image.new("RGBA", (4, 4), (1, 2, 3, 4))
    buf = io.BytesIO()
    rgba.save(buf, "PNG")
    with pytest.raises(ValueError, match="not indexed"):
        replace_palette(buf.getvalue(), bytes(3), bytes(1))


def test_replace_palette_validates_lengths():
    tile = np.zeros((2, 2), dtype=np.uint8)
    png = encode_indexed_png(tile, bytes(3), bytes(1))
    with pytest.raises(ValueError):
        replace_palette(png, bytes(4), None)
    with pytest.raises(ValueError):
        replace_palette(png, bytes(3), bytes(2))
    with pytest.raises(ValueError, match="signature"):
        replace_palette(b"nope" + png, bytes(3), None)


def test_iter_chunks_detects_truncation():
    good = b"\x89PNG\r\n\x1a\n" + make_chunk(b"IEND", b"")
    assert [t for t, _ in iter_chunks(good)] == [b"IEND"]
    with pytest.raises(ValueError, match="truncated"):
        list(iter_chunks(good[:-2]))
