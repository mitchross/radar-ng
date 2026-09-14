"""Release checks with isolated scratch files and no publication or notifications."""

import asyncio
import io
import os
from pathlib import Path
import tempfile

from temporalio import activity


def check_release() -> dict:
    import numpy as np
    from PIL import Image
    from backend.shared.png_palette import replace_palette

    role = os.environ.get("WORKER_ROLE", "legacy")
    if role not in {"mrms", "nowcast", "hrrr", "aux", "alerts", "legacy", "all"}:
        raise ValueError("Unknown worker role")
    palettes = os.environ.get("PALETTES", "classic,muted,vivid").split(",")
    palette_dir = Path(__file__).resolve().parents[2] / "backend/shared/palettes"
    for palette in palettes:
        if not (palette_dir / f"{palette.strip()}.json").is_file():
            raise ValueError("Configured palette is absent from the image")

    # Exercise the actual palette transformation and native imaging dependencies.
    source = Image.fromarray(np.array([[0, 1], [1, 0]], dtype=np.uint8), mode="P")
    source.putpalette([0, 0, 0, 255, 255, 255] + [0] * 762)
    buf = io.BytesIO()
    source.save(buf, format="PNG")
    transformed = replace_palette(buf.getvalue(), bytes([0, 0, 0, 255, 0, 0]), None)
    with Image.open(io.BytesIO(transformed)) as decoded:
        if decoded.convert("RGB").getpixel((1, 0)) != (255, 0, 0):
            raise ValueError("Palette fixture produced incorrect pixels")
    if role in {"nowcast", "legacy", "all"}:
        from pysteps import motion, nowcasts
        motion.get_method("LK")
        nowcasts.get_method("sprog")
    if role in {"hrrr", "aux", "legacy", "all"}:
        import pygrib
        if not pygrib.grib_api_version:
            raise ValueError("ecCodes is unavailable")

    for variable, default in (
        ("TILE_DIR", "/data/tiles"),
        ("GRID_DIR", "/data/grids"),
        ("STATE_DIR", "/data/state"),
    ):
        # Never create the mount root: a missing volume must fail the gate.
        with tempfile.TemporaryDirectory(prefix=".deployment-smoke-", dir=os.environ.get(variable, default)) as scratch:
            pending, published = Path(scratch) / "pending", Path(scratch) / "published"
            with pending.open("wb") as output:
                output.write(transformed)
                output.flush()
                os.fsync(output.fileno())
            pending.replace(published)
            if published.read_bytes() != transformed:
                raise ValueError("Volume round-trip failed")
    return {"ok": True, "role": role}


@activity.defn(name="radar_deployment_smoke")
async def radar_deployment_smoke() -> dict:
    return await asyncio.to_thread(check_release)
