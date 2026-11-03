#!/usr/bin/env python3
"""生成全部图标资源(可复现,改设计就改这个脚本):

  assets/menubar-connected.png     菜单栏图标·已连接(实心,模板图,46×36 = 23×18pt @2x)
  assets/menubar-disconnected.png  菜单栏图标·未连接(空心线框)
  assets/menubar-voice-dark.png    语音键按住时:白色机身 + 右上角黄点(深色菜单栏用,非模板)
  assets/menubar-voice-light.png   同上,黑色机身(浅色菜单栏用)
  assets/AppIcon.icns              App 图标(Finder / 关于)
  assets/AppIcon-1024.png          App 图标源图

图标几何以机身中心为原点(单位:@2x 像素)。画布比机身宽,机身两侧留出空白,
选中状态下系统画的高亮底不会贴着机身边缘。

依赖:Pillow、numpy、macOS 自带的 iconutil。用法: python3 scripts/make-icons.py
"""
import math
import subprocess
import tempfile
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "assets"

# ---------- 几何(机身中心为原点) ----------
BODY_HW, BODY_HH, BODY_R = 17.0, 11.0, 5.0      # 机身半宽、半高、圆角
PAD = 4.0                                        # 机身边缘到旋钮 / 按键的内边距
KNOB_R = 4.4
KNOB = (-BODY_HW + PAD + KNOB_R, 0.0)            # 旋钮在左,完整的圆
KEY_HALF, KEY_STEP, KEY_ROW = 1.9, 4.8, 3.3      # 按键半边长、列距、行偏移
_last = BODY_HW - PAD - KEY_HALF                 # 最右一列的中心,右内边距与左边对称
KEYS = [(_last - (2 - c) * KEY_STEP, (-1 + 2 * r) * KEY_ROW) for r in range(2) for c in range(3)]

# ---------- 菜单栏图标:有符号距离场 + 超采样抗锯齿 ----------
W, H, SS = 46, 36, 8
CX, CY = W / 2, H / 2


def rrect(x, y, cx, cy, hw, hh, r):
    qx, qy = abs(x - cx) - (hw - r), abs(y - cy) - (hh - r)
    return math.hypot(max(qx, 0), max(qy, 0)) + min(max(qx, qy), 0) - r  # <0 在内部


def circle(x, y, cx, cy, r):
    return math.hypot(x - cx, y - cy) - r


def keys_hit(x, y):
    return any(rrect(x, y, kx, ky, KEY_HALF, KEY_HALF, 0.7) < 0 for kx, ky in KEYS)


def connected(x, y):
    """实心机身,按键与旋钮镂空(旋钮是完整的圆)。x、y 是相对机身中心的坐标。"""
    if rrect(x, y, 0, 0, BODY_HW, BODY_HH, BODY_R) >= 0:
        return False
    return not (keys_hit(x, y) or circle(x, y, *KNOB, KNOB_R) < 0)


def disconnected(x, y):
    """空心线框:机身描边,按键实心,旋钮是完整的圆环。"""
    frame = -2.2 < rrect(x, y, 0, 0, BODY_HW, BODY_HH, BODY_R) < 0
    knob_ring = -2.0 < circle(x, y, *KNOB, KNOB_R) < 0
    return frame or keys_hit(x, y) or knob_ring


# 语音键按住时的指示:机身右上角的黄点,周围挖一圈空隙让它从机身上"浮"出来
DOT_C = (BODY_HW + 1.2, -BODY_HH - 0.2)   # 挂在机身外的右上角,不碰按键
DOT_R, DOT_GAP = 3.4, 1.5
YELLOW = (255, 204, 0)


def render_color(inside, rgb):
    """非模板彩色图标:机身用 rgb,右上角带黄点。画布尺寸与模板图标完全相同,所以切换时位置不会变。"""
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
                        pass  # 空隙
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


# ---------- App 图标 ----------
def app_icon(size=1024):
    S = size * 2  # 2x 超采样后再缩小

    # 超椭圆(squircle)遮罩
    ys, xs = np.mgrid[0:S, 0:S].astype(np.float32)
    nx, ny = (xs - S / 2) / (S / 2 * 0.98), (ys - S / 2) / (S / 2 * 0.98)
    mask = (np.abs(nx) ** 5 + np.abs(ny) ** 5 <= 1.0)
    mask_img = Image.fromarray((mask * 255).astype(np.uint8))

    # 背景:纵向深色渐变 + 顶部微弱高光
    t = np.linspace(0, 1, S, dtype=np.float32)[:, None]
    top, bot = np.array([62, 68, 84], np.float32), np.array([20, 22, 28], np.float32)
    grad = top[None, None, :] * (1 - t[..., None]) + bot[None, None, :] * t[..., None]
    grad = np.repeat(grad, S, axis=1)
    glow = np.exp(-(((xs - S / 2) / (S * .45)) ** 2 + ((ys - S * .05) / (S * .35)) ** 2))
    grad = np.clip(grad + glow[..., None] * 18, 0, 255).astype(np.uint8)
    bg = Image.fromarray(grad).convert("RGBA")
    bg.putalpha(mask_img)

    # 图形:亮色机身 + 深色按键 + 带蓝点的旋钮。整体缩放,四周留出 macOS 图标应有的留白
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
