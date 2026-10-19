#!/usr/bin/env python3
"""Draws the Finder background of Mugetsu.dmg: an eclipse between the two icons and an orbit
arc that carries the eye from the app to Applications. Writes dmg-background.png (660x400) and
dmg-background@2x.png next to this file. Needs Pillow, numpy and the Inter font.

Icon positions here must match dmg-settings.py. Finder draws the icon labels itself, black in
light mode and white in dark mode, so each label sits on a mid-grey plate readable in both."""
import math
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

W, H = 660, 400
APP, APPS = (170, 190), (490, 190)  # icon centres, as in dmg-settings.py
LABEL_Y = 272                       # centre of the label row under 128 px icons
SS = 2                              # supersample the 2x image, then downsample
FONT = "/usr/share/fonts/opentype/inter/Inter-Medium.otf"

BG = np.array([9, 6, 10]) / 255
RED = np.array([181, 18, 47]) / 255
GLOW = np.array([224, 96, 122]) / 255
INK = (240, 233, 234)
SOFT = (163, 149, 154)


def render(scale):
    w, h = W * scale, H * scale
    y, x = np.mgrid[0:h, 0:w].astype(np.float32) / scale  # logical coordinates
    img = np.broadcast_to(BG, (h, w, 3)).copy()

    # Faint vertical falloff and a red haze behind the eclipse.
    img += (1 - y / H)[..., None] * np.array([0.02, 0.012, 0.018])
    ex, ey, er = W / 2, 150, 44
    d = np.hypot(x - ex, y - ey)
    img += np.exp(-((d / 210) ** 2))[..., None] * RED * 0.22

    # Dust of stars, fixed seed so the image is reproducible.
    rng = np.random.default_rng(7)
    for sx, sy, b in zip(rng.uniform(0, W, 140), rng.uniform(0, H, 140), rng.uniform(0.15, 0.7, 140)):
        if math.hypot(sx - ex, sy - ey) < 80:
            continue
        x0, x1, y0, y1 = (int((v - 3) * scale) for v in (sx, sx + 6, sy, sy + 6))
        r = np.hypot(x[y0:y1, x0:x1] - sx, y[y0:y1, x0:x1] - sy)
        img[y0:y1, x0:x1] += (np.exp(-((r / 0.7) ** 2)) * b * 0.55)[..., None]

    # Corona: tight bright ring plus a wide soft halo, brighter on the crescent side (right).
    lean = np.clip((x - ex) / (d + 1e-3), -1, 1) * 0.5 + 0.5
    out = np.clip(d - er, 0, None)
    corona = np.exp(-out / 5) * (0.6 + 0.9 * lean) + np.exp(-out / 26) * 0.55 * (0.5 + lean)
    img += (corona * (d >= er))[..., None] * (RED * 0.85 + GLOW * 0.35)
    img += (np.exp(-out / 2.2) * lean ** 3 * (d >= er))[..., None] * np.array([1.0, 0.86, 0.88]) * 0.9

    # The moon: near-black disc with a sliver of cream light on its right edge, like the icon.
    disc = np.clip(er - d + 0.5, 0, 1)
    img = img * (1 - disc[..., None]) + disc[..., None] * np.array([0.012, 0.008, 0.01])
    inner = np.hypot(x - (ex - 6), y - ey)
    crescent = disc * np.clip(inner - (er - 1) + 0.5, 0, 1)
    img = img * (1 - crescent[..., None]) + crescent[..., None] * np.array([0.94, 0.91, 0.9])

    out_img = Image.fromarray((np.clip(img, 0, 1) * 255).astype(np.uint8), "RGB")
    draw_vectors(out_img, scale)
    return out_img


def draw_vectors(im, s):
    # Orbit: a dashed ellipse arc dipping under the eclipse, from the app's edge to Applications'.
    layer = Image.new("RGBA", im.size, (0, 0, 0, 0))
    g = ImageDraw.Draw(layer)
    cx, cy, rx, ry = W / 2, 150, 104, 80
    a0, a1 = math.radians(156), math.radians(30)  # through the bottom (y points down)
    pts, n = [], 240
    for i in range(n + 1):
        t = a0 - (a0 - a1) * i / n
        pts.append((cx + rx * math.cos(t), cy + ry * math.sin(t)))
    # Draw dashes, fading in from the app side.
    dash, gap, acc, on = 6, 5, 0.0, True
    for i in range(len(pts) - 1):
        (x0, y0), (x1, y1) = pts[i], pts[i + 1]
        seg = math.hypot(x1 - x0, y1 - y0)
        acc += seg
        if on:
            k = i / n
            alpha = int(255 * (0.25 + 0.75 * k))
            col = tuple(int(a * (1 - k) + b * k) for a, b in zip(SOFT, (255, 156, 174)))
            g.line([(x0 * s, y0 * s), (x1 * s, y1 * s)], fill=col + (alpha,), width=max(1, round(1.6 * s)))
        if acc >= (dash if on else gap):
            acc, on = 0.0, not on
    # Arrowhead at the end, pointing along the tangent.
    (xa, ya), (xb, yb) = pts[-6], pts[-1]
    ang = math.atan2(yb - ya, xb - xa)
    L, spread = 10, math.radians(26)
    tip = (xb + 3 * math.cos(ang), yb + 3 * math.sin(ang))
    wing = lambda sgn: (tip[0] - L * math.cos(ang + sgn * spread), tip[1] - L * math.sin(ang + sgn * spread))
    g.line([(p[0] * s, p[1] * s) for p in (wing(1), tip, wing(-1))], fill=(255, 156, 174, 255),
           width=max(1, round(1.8 * s)), joint="curve")
    glow = layer.filter(ImageFilter.GaussianBlur(3 * s))
    im.paste(glow, (0, 0), glow)
    im.paste(layer, (0, 0), layer)

    # Label plates: mid-grey (~#767076) so Finder's black or white label text both read at 4.5:1.
    plates = Image.new("RGBA", im.size, (0, 0, 0, 0))
    p = ImageDraw.Draw(plates)
    for (px, _), wd in ((APP, 86), (APPS, 104)):
        box = [(px - wd / 2) * s, (LABEL_Y - 10) * s, (px + wd / 2) * s, (LABEL_Y + 10) * s]
        p.rounded_rectangle(box, radius=10 * s, fill=(118, 111, 115, 255), outline=(140, 131, 136, 255),
                            width=max(1, s // 2))
    im.paste(plates, (0, 0), plates)

    # Caption.
    t = ImageDraw.Draw(im)
    font = ImageFont.truetype(FONT, 11 * s)
    spaced = "   ".join(" ".join(w) for w in "DRAG MUGETSU INTO APPLICATIONS".split())  # wide tracking
    tw = t.textlength(spaced, font=font)
    t.text(((W * s - tw) / 2, 344 * s), spaced, font=font, fill=SOFT)


if __name__ == "__main__":
    here = Path(__file__).parent
    big = render(2 * SS)
    big.resize((W * 2, H * 2), Image.LANCZOS).save(here / "dmg-background@2x.png", optimize=True)
    big.resize((W, H), Image.LANCZOS).save(here / "dmg-background.png", optimize=True)
