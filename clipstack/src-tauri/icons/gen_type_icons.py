#!/usr/bin/env python3
"""生成托盘历史条目的「分类图标」：与首界面侧边栏各分类所用 TypeIcon 一致（字形 + 颜色）。

颜色取自前端 src/lib/format.ts 的 TYPE_META（text/link/code/image），保证与首界面完全一致；
透明背景。以 64x64 绘制后缩到 32x32（@2x 适配视网膜），字形相对画布略缩小（SCALE<1）留白，
使菜单里图标比原尺寸小一点、更精致。
覆盖 text / link / code / image 四种（文件类型已在托盘历史中排除，未生成）。

坐标说明：TypeIcon 的 SVG viewBox 为 0..24，按 factor=SS/24 映射到 64 画布，再以 SCALE 围绕
画布中心缩小，产生留白（图标整体变小）。
"""
from PIL import Image, ImageDraw

SIZE = 32          # 输出尺寸（@2x）
SS = SIZE * 2      # 超采样绘制尺寸
F = SS / 24.0      # viewBox(24) -> 画布(64) 缩放因子
SCALE = 0.82       # 字形相对画布缩小比例（<1 → 图标更小、四周留白更多）
CX, CY = SS / 2, SS / 2
W = 4              # 描边宽度（画布像素）

# 与前端 src/lib/format.ts 的 TYPE_META.color 完全一致。
COLORS = {
    "text": (5, 150, 105, 255),    # #059669
    "link": (37, 99, 235, 255),    # #2563eb
    "code": (124, 58, 237, 255),   # #7c3aed
    "image": (234, 88, 12, 255),   # #ea580c
}


def new_canvas():
    return Image.new("RGBA", (SS, SS), (0, 0, 0, 0))


def p(x, y, scale=SCALE):
    """viewBox 坐标 -> 画布坐标（围绕中心缩小 scale）。"""
    px, py = x * F, y * F
    return (CX + (px - CX) * scale, CY + (py - CY) * scale)


def draw_text():
    """大写 T：竖干 + 顶部横杠 + 底部短横（对应 TypeIcon text）。
    文字类型图标整体再缩小 10%（SCALE*0.9），描边同步变细以保持比例。
    """
    s = SCALE * 0.9
    w = max(1, int(W * 0.9))
    img = new_canvas()
    d = ImageDraw.Draw(img)
    d.line([p(12, 5, s), p(12, 19, s)], fill=COLORS["text"], width=w, joint="curve")
    d.line([p(4, 6, s), p(20, 6, s)], fill=COLORS["text"], width=w, joint="curve")
    d.line([p(9, 19, s), p(15, 19, s)], fill=COLORS["text"], width=w, joint="curve")
    return img


def draw_code():
    """成对尖括号 < >（对应 TypeIcon code）。"""
    img = new_canvas()
    d = ImageDraw.Draw(img)
    d.line([p(8, 18), p(2, 12), p(8, 6)], fill=COLORS["code"], width=W, joint="curve")
    d.line([p(16, 6), p(22, 12), p(16, 18)], fill=COLORS["code"], width=W, joint="curve")
    return img


def draw_image():
    """圆角矩形画框 + 左上小圆（太阳）+ 右下山脉折线（对应 TypeIcon image）。"""
    img = new_canvas()
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([p(3, 3), p(21, 21)], radius=2 * F * SCALE, outline=COLORS["image"], width=W)
    cx, cy = p(8.5, 8.5)
    r = 1.5 * F * SCALE
    d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=COLORS["image"])
    d.line([p(21, 15), p(16, 10), p(5, 21)], fill=COLORS["image"], width=W, joint="curve")
    return img


def draw_capsule(angle_deg, color, cx_view, cy_view, w_view, h_view):
    """单个圆角胶囊（链节），中心位于 (cx_view, cy_view)，旋转 angle_deg 度。"""
    tmp = new_canvas()
    d = ImageDraw.Draw(tmp)
    w, h = w_view * F * SCALE, h_view * F * SCALE
    cx, cy = p(cx_view, cy_view)
    x0, y0 = cx - w / 2, cy - h / 2
    d.rounded_rectangle([x0, y0, x0 + w, y0 + h], radius=w / 2, outline=color, width=W)
    return tmp.rotate(angle_deg, resample=Image.BICUBIC, center=(SS / 2, SS / 2))


def draw_link():
    """两个互锁的圆角胶囊链节，与前端 TypeIcon link 的字形一致。"""
    base = new_canvas()
    # 上环（右上→左下）与下环（左下→右上）互锁，尺寸/角度与 TypeIcon link 路径对应。
    cap1 = draw_capsule(45, COLORS["link"], 15, 9, 10, 13)
    base = Image.alpha_composite(base, cap1)
    cap2 = draw_capsule(-45, COLORS["link"], 9, 15, 10, 13)
    base = Image.alpha_composite(base, cap2)
    return base


def save(img, name):
    img = img.resize((SIZE, SIZE), Image.LANCZOS)
    out = f"{name}.png"
    img.save(out)
    print("wrote", out, img.size)


if __name__ == "__main__":
    save(draw_text(), "menu-type-text")
    save(draw_link(), "menu-type-link")
    save(draw_code(), "menu-type-code")
    save(draw_image(), "menu-type-image")
