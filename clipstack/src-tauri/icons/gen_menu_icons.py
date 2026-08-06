#!/usr/bin/env python3
"""生成托盘菜单图标：menu-open.png（窗口）与 menu-settings.png（齿轮）。

中性灰 glyph（#9B9B9B），透明背景，深浅模式均可见。
以 64x64 绘制后降采样到 32x32（@2x 适配视网膜），带抗锯齿。
"""
from PIL import Image, ImageDraw

SIZE = 32          # 输出尺寸（@2x）
SS = SIZE * 2      # 超采样绘制尺寸
GRAY = (180, 180, 180, 255)


def new_canvas():
    return Image.new("RGBA", (SS, SS), (0, 0, 0, 0))


def draw_window():
    img = new_canvas()
    d = ImageDraw.Draw(img)
    # 窗口外框（圆角矩形）
    x0, y0, x1, y1 = 10, 12, 54, 54
    d.rounded_rectangle([x0, y0, x1, y1], radius=6, outline=GRAY, width=5)
    # 标题栏分隔线
    d.line([x0 + 4, y0 + 12, x1 - 4, y0 + 12], fill=GRAY, width=5)
    # 标题栏上的三个小圆点（交通灯示意）
    cy = y0 + 6
    for dx in (10, 22, 34):
        d.ellipse([x0 + dx - 3, cy - 3, x0 + dx + 3, cy + 3], fill=GRAY)
    return img


def draw_gear():
    import math
    img = new_canvas()
    d = ImageDraw.Draw(img)
    cx, cy = SS / 2, SS / 2
    outer = 24
    inner = 18
    teeth = 8
    # 齿轮主体：用多边形（外齿 + 内圆）近似
    pts = []
    for i in range(teeth * 2):
        ang = math.pi * i / teeth
        r = outer if i % 2 == 0 else inner
        pts.append((cx + r * math.cos(ang), cy + r * math.sin(ang)))
    d.polygon(pts, fill=GRAY)
    # 中心孔（透明）
    d.ellipse([cx - 8, cy - 8, cx + 8, cy + 8], fill=(0, 0, 0, 0))
    # 中心环描边增强辨识
    d.ellipse([cx - 8, cy - 8, cx + 8, cy + 8], outline=GRAY, width=3)
    return img


def save(img, name):
    img = img.resize((SIZE, SIZE), Image.LANCZOS)
    out = f"{name}.png"
    img.save(out)
    print("wrote", out, img.size)


if __name__ == "__main__":
    save(draw_window(), "menu-open")
    save(draw_gear(), "menu-settings")
