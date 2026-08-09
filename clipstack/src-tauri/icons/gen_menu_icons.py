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


def draw_info():
    """关于系统（info）：圆环 + 圆点 + 竖线，构成经典「i」字形。"""
    img = new_canvas()
    d = ImageDraw.Draw(img)
    cx, cy = SS / 2, SS / 2
    r = 22
    # 外圈圆环
    d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=GRAY, width=5)
    # 「i」的圆点（上方）
    dot_y = cy - 9
    d.ellipse([cx - 3, dot_y - 3, cx + 3, dot_y + 3], fill=GRAY)
    # 「i」的竖线（下方）
    d.line([cx, cy - 1, cx, cy + 13], fill=GRAY, width=5)
    return img


def draw_help():
    """帮助（问号圆圈）：使用与设置齿轮、关于系统相同的灰度色系，
    确保在托盘菜单的浅色 / 深色背景下均清晰可见。
    """
    img = new_canvas()
    d = ImageDraw.Draw(img)
    cx, cy = SS / 2, SS / 2
    r = 22
    # 外圈圆环
    d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=GRAY, width=5)
    # 问号上半弧（10 点到 2 点，经过 12 点）
    d.arc([cx - 11, cy - 14, cx + 11, cy + 8], start=240, end=300, fill=GRAY, width=5)
    # 问号竖线
    d.line([cx, cy - 2, cx, cy + 6], fill=GRAY, width=5)
    # 问号点
    d.ellipse([cx - 3, cy + 11, cx + 3, cy + 17], fill=GRAY)
    return img


def draw_lock():
    """锁定（锁）：顶部锁梁拱 + 锁体方框 + 锁孔，与窗口 / 齿轮风格一致的中性灰描边。"""
    img = new_canvas()
    d = ImageDraw.Draw(img)
    cx = SS / 2
    # 锁体方框（圆角矩形）
    x0, y0, x1, y1 = 16, 34, 48, 56
    d.rounded_rectangle([x0, y0, x1, y1], radius=5, outline=GRAY, width=5)
    # 锁梁拱（锁体上方的上半圆），底点落在锁体顶边两侧。
    d.arc([cx - 12, y0 - 12, cx + 12, y0], start=180, end=360, fill=GRAY, width=5)
    # 锁孔：上方小圆 + 向下短竖线，构成经典挂锁锁孔。
    hx, hy = cx, (y0 + y1) / 2 + 1
    d.ellipse([hx - 3, hy - 6, hx + 3, hy], fill=GRAY)
    d.line([hx, hy, hx, y1 - 5], fill=GRAY, width=4)
    return img


def save(img, name):
    img = img.resize((SIZE, SIZE), Image.LANCZOS)
    out = f"{name}.png"
    img.save(out)
    print("wrote", out, img.size)


if __name__ == "__main__":
    save(draw_window(), "menu-open")
    save(draw_gear(), "menu-settings")
    save(draw_info(), "menu-about")
    save(draw_help(), "menu-help")
    save(draw_lock(), "menu-lock")
