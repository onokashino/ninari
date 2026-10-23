"""Builds the app's icons from the stamp in crates/app/icons/source.

mark.svg is the stamp for large sizes, mark-small.svg the one for 16-48 px, which stays readable
in the tray and the taskbar. Writes the PNGs a release packs (crates/app/icons), the one inside
the program for the tray and notifications (crates/app/assets/icon.png), icon.ico for a Windows
build, and the site's copies of the stamp.

Needs rsvg-convert (librsvg) and Pillow. Run from anywhere:
    python scripts/build-icons.py
"""

import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "crates" / "app" / "icons"
SOURCE = ICONS / "source"
SMALL = (16, 20, 24, 32, 40, 48)
LARGE = (64, 128, 256, 512)


def render(svg: Path, size: int, out: Path) -> Image.Image:
    subprocess.run(
        ["rsvg-convert", "-w", str(size), "-h", str(size), "-o", str(out), str(svg)], check=True
    )
    return Image.open(out).convert("RGBA")


def main() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        frames = {s: render(SOURCE / "mark-small.svg", s, Path(tmp, f"{s}.png")) for s in SMALL}
        frames |= {s: render(SOURCE / "mark.svg", s, Path(tmp, f"{s}.png")) for s in LARGE}

    frames[32].save(ICONS / "32x32.png")
    frames[128].save(ICONS / "128x128.png")
    frames[256].save(ICONS / "128x128@2x.png")
    frames[512].save(ICONS / "icon.png")
    frames[128].save(ROOT / "crates" / "app" / "assets" / "icon.png")
    ico = [s for s in sorted(frames) if s <= 256]
    frames[256].save(
        ICONS / "icon.ico",
        format="ICO",
        sizes=[(s, s) for s in ico],
        append_images=[frames[s] for s in ico if s != 256],
    )
    shutil.copy(SOURCE / "mark.svg", ROOT / "site" / "assets" / "mark.svg")
    shutil.copy(SOURCE / "mark-small.svg", ROOT / "site" / "assets" / "favicon.svg")
    print("icons:", ", ".join(f"{s}px" for s in sorted(frames)))


if __name__ == "__main__":
    main()
