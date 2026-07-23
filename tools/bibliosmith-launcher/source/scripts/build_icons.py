#!/usr/bin/env python3
"""Render every launcher icon raster from the SVG master, then verify the result."""

from __future__ import annotations

import argparse
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

SOURCE_ROOT = Path(__file__).resolve().parent.parent
MASTER = SOURCE_ROOT / "assets" / "bibliosmith-launcher-icon.svg"
ICONS = SOURCE_ROOT / "src-tauri" / "icons"

# macOS draws an app icon on a 1024 canvas with the rounded tile occupying 824
# of it, leaving room for the shadow the system composites underneath. An icon
# that fills the whole canvas renders ~24% larger than every native app in the
# Dock, which is exactly the bug this script exists to keep from coming back.
TILE_RATIO = 824 / 1024
TILE_TOLERANCE = 0.015

# Each of these ships at @1x and @2x, giving iconutil the ten representations
# a complete .icns carries.
ICNS_BASES = (16, 32, 128, 256, 512)
ICO_FRAMES = (16, 24, 32, 48, 64, 128, 256)

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"

# Everything below is derived from MASTER; nothing here is hand-drawn.
RASTERS: dict[Path, int] = {
    ICONS / "32x32.png": 32,
    ICONS / "128x128.png": 128,
    ICONS / "128x128@2x.png": 256,
    ICONS / "icon.png": 512,
    SOURCE_ROOT / "assets" / "bibliosmith-launcher-icon.png": 512,
    SOURCE_ROOT / "public" / "favicon.png": 256,
}


class BuildError(RuntimeError):
    pass


def require_tools() -> None:
    for tool in ("rsvg-convert", "iconutil"):
        if subprocess.run(["which", tool], capture_output=True).returncode != 0:
            raise BuildError(
                f"{tool} not found on PATH. Install librsvg (brew install librsvg); "
                "iconutil ships with macOS."
            )
    try:
        import PIL  # noqa: F401
    except ImportError as exc:  # pragma: no cover - environment dependent
        raise BuildError("Pillow is required (pip install Pillow).") from exc


def render(size: int, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["rsvg-convert", "-w", str(size), "-h", str(size), str(MASTER), "-o", str(dest)],
        check=True,
        capture_output=True,
    )


def build_icns(dest: Path) -> None:
    """iconutil refuses anything that is not a well-formed .iconset directory."""
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "icon.iconset"
        iconset.mkdir()
        for base in ICNS_BASES:
            render(base, iconset / f"icon_{base}x{base}.png")
            render(base * 2, iconset / f"icon_{base}x{base}@2x.png")
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(dest)],
            check=True,
            capture_output=True,
        )


def build_ico(dest: Path) -> None:
    """Write PNG-compressed frames.

    ImageMagick's icon:auto-resize stores every frame as a raw bitmap, which
    inflates this file more than tenfold. Rendering each frame from the master
    at its native size also beats downsampling a single large raster.
    """
    from PIL import Image

    with tempfile.TemporaryDirectory() as tmp:
        frames = []
        for size in ICO_FRAMES:
            path = Path(tmp) / f"{size}.png"
            render(size, path)
            frames.append(Image.open(path).convert("RGBA"))
        frames[-1].save(
            dest,
            format="ICO",
            sizes=[(s, s) for s in ICO_FRAMES],
            append_images=frames[:-1],
        )


def tile_ratio(path: Path, page: int | None = None) -> float:
    """Width of the opaque tile relative to the canvas, ignoring the soft shadow."""
    from PIL import Image

    image = Image.open(path)
    if page is not None:
        image.size = (page, page)
        image.load()
    image = image.convert("RGBA")
    opaque = image.split()[3].point(lambda v: 255 if v > 250 else 0)
    box = opaque.getbbox()
    if box is None:
        raise BuildError(f"{path.name} is fully transparent")
    return (box[2] - box[0]) / image.size[0]


def ico_frames_are_png(path: Path) -> bool:
    blob = path.read_bytes()
    count = struct.unpack_from("<H", blob, 4)[0]
    for index in range(count):
        offset = struct.unpack_from("<I", blob, 6 + index * 16 + 12)[0]
        if blob[offset : offset + 8] != PNG_SIGNATURE:
            return False
    return True


def verify() -> list[str]:
    problems: list[str] = []

    for path, size in RASTERS.items():
        if not path.exists():
            problems.append(f"missing {path.relative_to(SOURCE_ROOT)}")
            continue
        ratio = tile_ratio(path)
        if abs(ratio - TILE_RATIO) > TILE_TOLERANCE:
            problems.append(
                f"{path.relative_to(SOURCE_ROOT)} tile is {ratio:.1%} of the canvas, "
                f"expected {TILE_RATIO:.1%}"
            )

    icns = ICONS / "icon.icns"
    if not icns.exists():
        problems.append("missing src-tauri/icons/icon.icns")
    else:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "icon.iconset"
            result = subprocess.run(
                ["iconutil", "-c", "iconset", str(icns), "-o", str(out)],
                capture_output=True,
            )
            if result.returncode != 0:
                problems.append("icon.icns cannot be read back by iconutil")
            else:
                found = len(list(out.glob("*.png")))
                expected = len(ICNS_BASES) * 2
                if found != expected:
                    problems.append(
                        f"icon.icns carries {found} representations, expected {expected}"
                    )

    ico = ICONS / "icon.ico"
    if not ico.exists():
        problems.append("missing src-tauri/icons/icon.ico")
    else:
        if not ico_frames_are_png(ico):
            problems.append("icon.ico has raw bitmap frames; they should all be PNG")
        if abs(tile_ratio(ico, page=256) - TILE_RATIO) > TILE_TOLERANCE:
            problems.append("icon.ico 256px frame is off the 824/1024 grid")

    return problems


def build() -> None:
    for path, size in RASTERS.items():
        render(size, path)
        print(f"  {path.relative_to(SOURCE_ROOT)}  {size}px")
    build_icns(ICONS / "icon.icns")
    print(f"  src-tauri/icons/icon.icns  {len(ICNS_BASES) * 2} representations")
    build_ico(ICONS / "icon.ico")
    print(f"  src-tauri/icons/icon.ico  {len(ICO_FRAMES)} PNG frames")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify the committed rasters without rewriting them",
    )
    args = parser.parse_args()

    try:
        require_tools()
        if not MASTER.exists():
            raise BuildError(f"SVG master not found at {MASTER}")

        if not args.check:
            print(f"Rendering from {MASTER.relative_to(SOURCE_ROOT)}")
            build()

        problems = verify()
    except BuildError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    except subprocess.CalledProcessError as exc:
        print(f"error: {exc.cmd[0]} failed: {exc.stderr.decode().strip()}", file=sys.stderr)
        return 1

    if problems:
        print("\nverification failed:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1

    print("\nAll icons verified against the macOS 824/1024 grid.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
