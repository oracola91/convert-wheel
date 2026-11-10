#!/usr/bin/env python3
"""Draws every app icon file in src-tauri/icons from the Pirouette design: a white
ring with a deep purple dot on a pink-to-orange rounded square, and the one-colour
menu bar glyph. Run it again after changing a number below; it overwrites the files.

    python3 scripts/make-icons.py

Needs Pillow (pip install pillow), ImageMagick's `magick` (for icon.ico) and, on
macOS, `iconutil` (for icon.icns). Geometry is on a 1024 grid, as in the design: the
rounded square sits 100 in from each side, so macOS shows it at the size of its own
icons, with room for the shadow.
"""
import os
import shutil
import subprocess
import sys
import tempfile

from PIL import Image, ImageChops, ImageDraw, ImageFilter

ICONS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "icons")

PINK = (0xFF, 0x6F, 0x91)
ORANGE = (0xFF, 0xA5, 0x52)
PURPLE = (0x3B, 0x2A, 0x6B)
SHADOW = (0x6B, 0x1F, 0x2E)

# Ring and dot on the 1024 grid. Small sizes get a thicker ring and a bigger dot so
# they stay readable, and at 16 and 24 px the dot loses its white edge.
FULL = {"ring": 230, "stroke": 64, "dot": 66, "edge": 16, "effects": True}
PLAIN = dict(FULL, effects=False)
SMALL = {"ring": 230, "stroke": 72, "dot": 74, "edge": 16, "effects": False}
TINY = {"ring": 230, "stroke": 84, "dot": 84, "edge": 0, "effects": False}
DOT_AT = (674.6, 349.4)  # on the ring, towards the top right


def style_for(px):
    if px <= 24:
        return TINY
    if px <= 48:
        return SMALL
    if px <= 64:
        return PLAIN
    return FULL


def disc(size, cx, cy, r):
    m = Image.new("L", (size, size), 0)
    ImageDraw.Draw(m).ellipse([cx - r, cy - r, cx + r, cy + r], fill=255)
    return m


def shadow(alpha, k, dy, blur, opacity):
    """A drop shadow of `alpha`, moved down by dy and blurred, both on the 1024 grid."""
    s = alpha.point(lambda v: int(v * opacity))
    s = ImageChops.offset(s, 0, int(round(dy * k)))
    s = s.filter(ImageFilter.GaussianBlur(blur * k))
    layer = Image.new("RGBA", alpha.size, SHADOW + (0,))
    layer.putalpha(s)
    return layer


def app_icon(px):
    st = style_for(px)
    S = max(px * 8, 2048)  # drawn large, then scaled down for smooth edges
    k = S / 1024
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # The rounded square with its diagonal gradient: pink top left, orange bottom right.
    x0, x1 = round(100 * k), round(924 * k)
    side = x1 - x0
    square = Image.new("L", (S, S), 0)
    ImageDraw.Draw(square).rounded_rectangle([x0, x0, x1 - 1, x1 - 1], radius=185 * k, fill=255)
    down = Image.linear_gradient("L").resize((side, side), Image.BILINEAR)
    across = down.rotate(90)  # dark on the left, light on the right
    ramp = ImageChops.add(down, across, scale=2.0)  # (x + y) / 2
    fill = Image.composite(Image.new("RGB", (side, side), ORANGE), Image.new("RGB", (side, side), PINK), ramp)
    face = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    face.paste(fill, (x0, x0))
    face.putalpha(square)

    if st["effects"]:
        img.alpha_composite(shadow(square, k, 12, 14, 0.28))
    img.alpha_composite(face)

    if st["effects"]:
        # A soft light on the top half: white at 26% fading out halfway down.
        glow = Image.linear_gradient("L").resize((side, side), Image.BILINEAR)
        glow = glow.point(lambda v: int(max(0.0, 1 - (v / 255) * 2) * 0.26 * 255))
        light = Image.new("L", (S, S), 0)
        light.paste(glow, (x0, x0))
        top = Image.new("RGBA", (S, S), (255, 255, 255, 0))
        top.putalpha(ImageChops.multiply(light, square))
        img.alpha_composite(top)

    # Ring and dot.
    c = 512 * k
    r_out = (st["ring"] + st["stroke"] / 2) * k
    r_in = (st["ring"] - st["stroke"] / 2) * k
    ring = ImageChops.subtract(disc(S, c, c, r_out), disc(S, c, c, r_in))
    dx, dy = DOT_AT[0] * k, DOT_AT[1] * k
    if st["edge"]:
        edge = disc(S, dx, dy, (st["dot"] + st["edge"] / 2) * k)
        inner = disc(S, dx, dy, (st["dot"] - st["edge"] / 2) * k)
        whites = ImageChops.lighter(ring, edge)
    else:
        inner = disc(S, dx, dy, st["dot"] * k)
        whites = ring
    if st["effects"]:
        img.alpha_composite(shadow(ImageChops.lighter(whites, inner), k, 8, 10, 0.25))
    white = Image.new("RGBA", (S, S), (255, 255, 255, 0))
    white.putalpha(whites)
    img.alpha_composite(white)
    purple = Image.new("RGBA", (S, S), PURPLE + (0,))
    purple.putalpha(inner)
    img.alpha_composite(purple)

    return img.resize((px, px), Image.LANCZOS)


def tray_glyph(px=36):
    """The menu bar glyph, black on clear: macOS tints it for light and dark menu bars.
    A ring with the dot cut free of it at the top right. 36 px = 18 pt on a Retina
    screen, the menu bar's size."""
    S = px * 16
    a = Image.new("L", (S, S), 0)
    d = ImageDraw.Draw(a)
    c, r_out, stroke = S / 2, S * 0.40, S * 0.14
    d.ellipse([c - r_out, c - r_out, c + r_out, c + r_out], fill=255)
    r_in = r_out - stroke
    d.ellipse([c - r_in, c - r_in, c + r_in, c + r_in], fill=0)
    dx, dy = c + S * 0.33 * 0.7071, c - S * 0.33 * 0.7071
    dr, gap = S * 0.135, S * 0.07
    d.ellipse([dx - dr - gap, dy - dr - gap, dx + dr + gap, dy + dr + gap], fill=0)
    d.ellipse([dx - dr, dy - dr, dx + dr, dy + dr], fill=255)
    out = Image.new("RGBA", (px, px), (0, 0, 0, 0))
    out.putalpha(a.resize((px, px), Image.LANCZOS))
    return out


def main():
    sizes = {}

    def icon(px):
        if px not in sizes:
            sizes[px] = app_icon(px)
        return sizes[px]

    for name, px in [("32x32.png", 32), ("64x64.png", 64), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 512)]:
        icon(px).save(os.path.join(ICONS, name))
    tray_glyph().save(os.path.join(ICONS, "tray.png"))

    work = tempfile.mkdtemp()
    try:
        # Windows: one .ico holding each size Explorer asks for.
        parts = []
        for px in (16, 24, 32, 48, 64, 256):
            p = os.path.join(work, f"{px}.png")
            icon(px).save(p)
            parts.append(p)
        subprocess.run(["magick", *parts, os.path.join(ICONS, "icon.ico")], check=True)

        # macOS: an iconset folder turned into icon.icns.
        if shutil.which("iconutil"):
            iconset = os.path.join(work, "icon.iconset")
            os.makedirs(iconset)
            for pt in (16, 32, 128, 256, 512):
                icon(pt).save(os.path.join(iconset, f"icon_{pt}x{pt}.png"))
                icon(pt * 2).save(os.path.join(iconset, f"icon_{pt}x{pt}@2x.png"))
            subprocess.run(["iconutil", "-c", "icns", iconset, "-o", os.path.join(ICONS, "icon.icns")], check=True)
        else:
            print("iconutil missing (not macOS): icon.icns left as it was", file=sys.stderr)
    finally:
        shutil.rmtree(work)
    print("icons written to", os.path.normpath(ICONS))


if __name__ == "__main__":
    main()
