#!/usr/bin/env python3
"""Generates all icon assets. Reproducible, so change this script to change the design.

  assets/menubar-connected.png     Menu bar icon, connected (solid, template image,
                                   46×36 = 23×18pt @2x)
  assets/menubar-disconnected.png  Menu bar icon, disconnected (hollow outline)
  assets/menubar-voice-dark.png    While the voice key is held: white body plus a yellow dot at
                                   the top right (for a dark menu bar, not a template)
  assets/menubar-voice-light.png   Same, with a black body (for a light menu bar)
  assets/AppIcon.icns              App icon (Finder and About window)
  assets/AppIcon-1024.png          Source image of the app icon

Icon geometry has its origin at the body center, in @2x pixels. The canvas is wider than the body
and leaves empty space on both sides of it, so the highlight the system draws behind a selected
menu bar item does not touch the body edge.

Needs Pillow, numpy and iconutil, which ships with macOS. Usage: python3 scripts/make-icons.py
"""
import math
import subprocess
import tempfile
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "assets"

# ---------- Geometry, origin at the body center ----------
BODY_HW, BODY_HH, BODY_R = 17.0, 11.0, 5.0      # body half width, half height, corner radius
PAD = 4.0                                        # inset from the body edge to the knob and keys
KNOB_R = 4.4
KNOB = (-BODY_HW + PAD + KNOB_R, 0.0)            # knob on the left, a full circle
KEY_HALF, KEY_STEP, KEY_ROW = 1.9, 4.8, 3.3      # key half side, column pitch, row offset
_last = BODY_HW - PAD - KEY_HALF                 # rightmost column center, mirrors the left inset
KEYS = [(_last - (2 - c) * KEY_STEP, (-1 + 2 * r) * KEY_ROW) for r in range(2) for c in range(3)]

# ---------- Menu bar icons, signed distance field plus supersampled antialiasing ----------
W, H, SS = 46, 36, 8
CX, CY = W / 2, H / 2


def rrect(x, y, cx, cy, hw, hh, r):
    qx, qy = abs(x - cx) - (hw - r), abs(y - cy) - (hh - r)
    return math.hypot(max(qx, 0), max(qy, 0)) + min(max(qx, qy), 0) - r  # negative means inside


def circle(x, y, cx, cy, r):
    return math.hypot(x - cx, y - cy) - r


def keys_hit(x, y):
    return any(rrect(x, y, kx, ky, KEY_HALF, KEY_HALF, 0.7) < 0 for kx, ky in KEYS)


def connected(x, y):
    """Solid body with the keys and the knob cut out, the knob being a full circle.
    x and y are relative to the body center."""
    if rrect(x, y, 0, 0, BODY_HW, BODY_HH, BODY_R) >= 0:
        return False
    return not (keys_hit(x, y) or circle(x, y, *KNOB, KNOB_R) < 0)


def disconnected(x, y):
    """Hollow outline: the body is stroked, the keys are solid, the knob is a full ring."""
    frame = -2.2 < rrect(x, y, 0, 0, BODY_HW, BODY_HH, BODY_R) < 0
    knob_ring = -2.0 < circle(x, y, *KNOB, KNOB_R) < 0
    return frame or keys_hit(x, y) or knob_ring


# Indicator while the voice key is held: a yellow dot at the top right, with a gap cut out around it so it floats off the body
DOT_C = (BODY_HW + 1.2, -BODY_HH - 0.2)   # off the top right of the body, clear of the keys
DOT_R, DOT_GAP = 3.4, 1.5
YELLOW = (255, 204, 0)


def render_color(inside, rgb):
    """Non-template color icon, body in rgb with a yellow dot at the top right.
    The canvas size is identical to the template icons, so nothing shifts when switching."""
    img = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    px = img.load()
    total = SS * SS
    for j in range(H):
        for i in range(W):
            ng = nd = 0
            for sj in range(SS):
                for si in range(SS):
                    x, y = i + (si + .5) / SS - CX, j + (sj + .5) / SS - CY
                    d = circle(x, y, *DOT_C, DOT_R)
                    if d < 0:
                        nd += 1
                    elif d < DOT_GAP:
                        pass  # the gap
                    elif inside(x, y):
                        ng += 1
            n = ng + nd
            if n:
                col = tuple(round((rgb[k] * ng + YELLOW[k] * nd) / n) for k in range(3))
                px[i, j] = (*col, round(255 * n / total))
    return img


def render_template(inside):
    img = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    px = img.load()
    for j in range(H):
        for i in range(W):
            hit = sum(
                inside(i + (si + .5) / SS - CX, j + (sj + .5) / SS - CY)
                for sj in range(SS) for si in range(SS)
            )
            px[i, j] = (0, 0, 0, round(255 * hit / (SS * SS)))
    return img


# ---------- App icon ----------
def app_icon(size=1024):
    S = size * 2  # supersample at 2x, then scale down

    # Squircle mask
    ys, xs = np.mgrid[0:S, 0:S].astype(np.float32)
    nx, ny = (xs - S / 2) / (S / 2 * 0.98), (ys - S / 2) / (S / 2 * 0.98)
    mask = (np.abs(nx) ** 5 + np.abs(ny) ** 5 <= 1.0)
    mask_img = Image.fromarray((mask * 255).astype(np.uint8))

    # Background: dark vertical gradient plus a faint highlight at the top
    t = np.linspace(0, 1, S, dtype=np.float32)[:, None]
    top, bot = np.array([62, 68, 84], np.float32), np.array([20, 22, 28], np.float32)
    grad = top[None, None, :] * (1 - t[..., None]) + bot[None, None, :] * t[..., None]
    grad = np.repeat(grad, S, axis=1)
    glow = np.exp(-(((xs - S / 2) / (S * .45)) ** 2 + ((ys - S * .05) / (S * .35)) ** 2))
    grad = np.clip(grad + glow[..., None] * 18, 0, 255).astype(np.uint8)
    bg = Image.fromarray(grad).convert("RGBA")
    bg.putalpha(mask_img)

    # Artwork: light body, dark keys, and a knob with a blue dot. Scaled down overall to leave the margin that macOS icons are expected to have
    f = S / 36.0 * 0.74
    g = ImageDraw.Draw(bg)

    def box(lx, ly, hw, hh):
        return [S / 2 + (lx - hw) * f, S / 2 + (ly - hh) * f, S / 2 + (lx + hw) * f, S / 2 + (ly + hh) * f]

    g.rounded_rectangle(box(0, 0, BODY_HW, BODY_HH), radius=BODY_R * f, fill=(238, 240, 245, 255))
    for kx, ky in KEYS:
        g.rounded_rectangle(box(kx, ky, KEY_HALF, KEY_HALF), radius=0.7 * f, fill=(40, 44, 56, 255))
    g.ellipse(box(*KNOB, KNOB_R, KNOB_R), fill=(40, 44, 56, 255))
    g.ellipse(box(*KNOB, 1.5, 1.5), fill=(96, 150, 255, 255))

    return bg.resize((size, size), Image.LANCZOS)


def main():
    OUT.mkdir(exist_ok=True)
    render_template(connected).save(OUT / "menubar-connected.png")
    render_template(disconnected).save(OUT / "menubar-disconnected.png")
    render_color(connected, (255, 255, 255)).save(OUT / "menubar-voice-dark.png")
    render_color(connected, (0, 0, 0)).save(OUT / "menubar-voice-light.png")

    icon = app_icon(1024)
    icon.save(OUT / "AppIcon-1024.png")
    with tempfile.TemporaryDirectory() as td:
        iconset = Path(td) / "AppIcon.iconset"
        iconset.mkdir()
        for s in (16, 32, 128, 256, 512):
            icon.resize((s, s), Image.LANCZOS).save(iconset / f"icon_{s}x{s}.png")
            icon.resize((s * 2, s * 2), Image.LANCZOS).save(iconset / f"icon_{s}x{s}@2x.png")
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(OUT / "AppIcon.icns")], check=True)
    print("generated:", ", ".join(sorted(p.name for p in OUT.iterdir())))


if __name__ == "__main__":
    main()
