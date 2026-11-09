#!/usr/bin/env python3
"""Compare the bounded adapter with Py-ART on locally downloaded real Archive II volumes."""
import argparse
import bz2
import os
import resource
import sys
import importlib.metadata
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

import numpy as np


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inspect", required=True, type=Path)
    parser.add_argument("volumes", nargs="+", type=Path)
    args = parser.parse_args()
    package = importlib.metadata.distribution("arm_pyart")
    source = package.locate_file("pyart/io/nexrad_level2.py")
    spec = importlib.util.spec_from_file_location("pyart_reference", source)
    reference = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(reference)
    print(f"reference=arm_pyart-{package.version} numpy={np.__version__}", flush=True)
    for path in args.volumes:
        station = path.name[:4]
        with tempfile.TemporaryFile() as output:
            subprocess.run([str(args.inspect.resolve()), station, str(path)], stdout=output, check=True)
            output.seek(0)
            actual = json.load(output)
        radar = reference.NEXRADLevel2File(str(path))
        groups = []
        current = []
        for ray in radar.radial_records:
            status = ray["msg_header"]["radial_spacing"]
            if status in (0, 3, 5):
                assert not current, "reference sweep interrupted"
            current.append(ray)
            if status in (2, 4):
                groups.append(current)
                current = []
        assert not current, "reference incomplete sweep"
        lowest = min(float(np.median([r["msg_header"]["elevation_angle"] for r in g])) for g in groups if any("REF" in r for r in g))
        expected = [(i + 1, g) for i, g in enumerate(groups) if any("REF" in r for r in g) and abs(float(np.median([r["msg_header"]["elevation_angle"] for r in g])) - lowest) <= 0.15]
        assert len(actual) == len(expected), (path, len(actual), len(expected))
        count = 0
        widths = set()
        missing = set()
        ref_min = float("inf")
        ref_max = float("-inf")
        for sweep, (ordinal, rays) in zip(actual, expected, strict=True):
            assert sweep["ordinal"] == ordinal
            assert len(sweep["rays"]) == len(rays)
            volume = rays[0]["VOL"]
            site = sweep["site"]
            assert np.float32(site["latitude_deg"]) == np.float32(volume["lat"])
            assert np.float32(site["longitude_deg"]) == np.float32(volume["lon"])
            assert site["site_height_m_raw"] == volume["height"]
            assert site["tower_height_m_raw"] == volume["feedhorn_height"]
            for got, ray in zip(sweep["rays"], rays, strict=True):
                header = ray["msg_header"]
                assert got["number"] == header["azimuth_number"]
                assert got["radial_status"] == header["radial_spacing"]
                assert got["blanking"] == header["radial_blanking"]
                assert np.float32(got["azimuth_deg"]) == np.float32(header["azimuth_angle"])
                assert np.float32(got["elevation_deg"]) == np.float32(header["elevation_angle"])
                from datetime import datetime, timedelta, timezone
                time = datetime(1970, 1, 1, tzinfo=timezone.utc) + timedelta(days=header["collect_date"] - 1, milliseconds=header["collect_ms"])
                assert datetime.fromisoformat(got["time"].replace("Z", "+00:00")) == time
                for field, moment in zip(("REF", "RHO", "ZDR"), got["moments"], strict=True):
                    if field not in ray:
                        assert moment is None
                        missing.add(field)
                        continue
                    data = ray[field]
                    assert moment is not None
                    for our, theirs in (("first_gate_m", "first_gate"), ("spacing_m", "gate_spacing"), ("word_bits", "word_size"), ("scale", "scale"), ("offset", "offset"), ("control_flags", "flags"), ("snr_threshold_raw", "snr_thres"), ("tover_raw", "thresh")):
                        assert moment[our] == data[theirs], (path, field, our)
                    widths.add(data["word_size"])
                    raw = np.asarray(data["data"])
                    expected_qc = np.where(raw == 0, 1, np.where(raw == 1, 2, 0)).astype(np.uint8)
                    if got["blanking"]:
                        expected_qc |= 4
                    np.testing.assert_array_equal(moment["qc"], expected_qc)
                    values = (raw.astype(np.float32) - data["offset"]) / data["scale"]
                    numeric = expected_qc == 0
                    got_values = np.array([np.nan if x is None else x for x in moment["values"]], dtype=np.float32)
                    np.testing.assert_array_equal(np.isnan(got_values), ~numeric)
                    np.testing.assert_allclose(got_values[numeric], values[numeric], rtol=0, atol=2e-5)
                    if field == "REF" and np.any(numeric):
                        ref_min = min(ref_min, float(np.min(got_values[numeric])))
                        ref_max = max(ref_max, float(np.max(got_values[numeric])))
                    count += len(raw)
        print(f"PASS {path.name} vcp={radar.vcp['msg5_header']['pattern_number']} low_cuts={[s["ordinal"] for s in actual]} gates={count} word_bits={sorted(widths)} absent={sorted(missing)} REF_min={ref_min} REF_max={ref_max}", flush=True)

    adversarial(args.inspect,args.volumes[0])


def reference_reader():
    package=importlib.metadata.distribution("arm_pyart")
    spec=importlib.util.spec_from_file_location("pyart_reference",package.locate_file("pyart/io/nexrad_level2.py"))
    module=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def bound_reference():
    resource.setrlimit(resource.RLIMIT_AS,(1024**3,1024**3))
    resource.setrlimit(resource.RLIMIT_CPU,(10,10))


def adversarial(inspect,source):
    original=source.read_bytes()
    corrupt=bytearray(original)
    corrupt[28]^=255
    expanded=bz2.compress(bytes(8*1024*1024+1))
    bomb=original[:24]+len(expanded).to_bytes(4,"big")+expanded
    cases={"corrupt":corrupt,"truncated":original[:len(original)//2],"record_bomb":bomb}
    with tempfile.TemporaryDirectory(prefix="level2-parity-") as directory:
        for name,data in cases.items():
            path=Path(directory)/name
            path.write_bytes(data)
            ours=subprocess.run([str(inspect.resolve()),source.name[:4],str(path)],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,timeout=15)
            assert ours.returncode!=0, f"adapter accepted {name}"
            reference=subprocess.run([sys.executable,str(Path(__file__).resolve()),"--reference-check",str(path)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=15,preexec_fn=bound_reference,env={**os.environ,"OPENBLAS_NUM_THREADS":"1"})
            assert reference.returncode in (0,2),f"reference resource limit or crash for {name}: {reference.returncode}"
            print(f"PASS adversarial={name} adapter=rejected reference={'accepted_partial' if reference.returncode==0 else 'rejected'}",flush=True)


if __name__ == "__main__":
    if len(sys.argv)>1 and sys.argv[1]=="--reference-check":
        try:
            reference_reader().NEXRADLevel2File(sys.argv[2])
        except Exception:
            raise SystemExit(2)
    else:
        main()
