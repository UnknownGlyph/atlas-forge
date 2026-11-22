"""Renders the Atlas Forge program icon from its vector source.

The icon is hand-written SVG in assets/icon/: atlas-forge.svg for 40 px and
up, atlas-forge-small.svg (no graticule or spark, thicker lines) for 16-32 px.
This script turns them into the files the program uses:

    atlas-forge.ico       Windows file icon, 16-256 px (build.rs puts it in atlas.exe)
    atlas-forge-256.png   window and taskbar icon (main.rs)
    atlas-forge-64.png    Creative Studio header and start card
    atlas-forge-1024.png  large preview

    python tools/make_icon.py

Needs Pillow and an SVG renderer: `pip install resvg-py` (or cairosvg).
"""
import io
import os

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
ICON = os.path.join(HERE, "..", "assets", "icon")
SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)


def renderer():
    try:
        import resvg_py

        return lambda svg, n: bytes(resvg_py.svg_to_bytes(svg_string=svg, width=n, height=n))
    except ImportError:
        pass
    try:
        import cairosvg

        return lambda svg, n: cairosvg.svg2png(bytestring=svg.encode("utf-8"), output_width=n, output_height=n)
    except ImportError:
        raise SystemExit("Needs an SVG renderer: pip install resvg-py  (or: pip install cairosvg)")


def main():
    render = renderer()

    def image(name, n):
        with open(os.path.join(ICON, name), encoding="utf-8") as f:
            return Image.open(io.BytesIO(render(f.read(), n))).convert("RGBA")

    for n in (1024, 256, 64):
        image("atlas-forge.svg", n).save(os.path.join(ICON, f"atlas-forge-{n}.png"), optimize=True)
    frames = {n: image("atlas-forge-small.svg" if n <= 32 else "atlas-forge.svg", n) for n in SIZES}
    frames[256].save(
        os.path.join(ICON, "atlas-forge.ico"),
        format="ICO",
        sizes=[(n, n) for n in SIZES],
        append_images=[frames[n] for n in SIZES if n != 256],
    )
    print("icon written to", os.path.abspath(ICON))


if __name__ == "__main__":
    main()
