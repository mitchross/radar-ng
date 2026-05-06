from pathlib import Path


def requirement_lines() -> list[str]:
    path = Path(__file__).with_name("requirements.txt")
    return [
        line.strip()
        for line in path.read_text().splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]


def test_nowcast_pins_numpy_below_two_for_opencv_abi():
    lines = requirement_lines()

    assert any(line.startswith("numpy") and "<2" in line for line in lines)


def test_nowcast_uses_recent_opencv_headless_build():
    lines = requirement_lines()

    assert any(
        line.startswith("opencv-python-headless")
        and ("4.10" in line or "4.11" in line or "4.12" in line)
        for line in lines
    )

