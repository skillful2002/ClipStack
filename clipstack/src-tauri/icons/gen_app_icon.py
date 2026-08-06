"""生成 ClipStack 应用图标源（1024x1024 PNG），供 `tauri icon` 生成各平台图标。

设计：蓝紫渐变圆角底板 + 白色剪贴板（卡舌 + 主体 + 文本行）+ 后方两张半透明
卡片体现「栈/Stack」含义。超采样 2x 后降采样抗锯齿。

用法：python3 gen_app_icon.py  -> 输出 src-tauri/icons/icon-source.png
"""

from PIL import Image, ImageDraw

S = 1024            # 最终尺寸
SS = 2048           # 超采样
scale = SS / S


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


C_TOP = (91, 141, 239)     # #5B8DEF 蓝
C_BOTTOM = (138, 91, 239)  # #8A5BEF 紫


def rr(draw, box, r, fill, outline=None, width=0):
    draw.rounded_rectangle(box, radius=int(r * scale), fill=fill, outline=outline, width=width)


# 渐变底板
img = Image.new("RGBA", (SS, SS), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
for y in range(SS):
    t = y / (SS - 1)
    col = lerp(C_TOP, C_BOTTOM, t)
    d.line([(0, y), (SS, y)], fill=col + (255,))

# 圆角裁切（整图作为图标）
radius = int(220 * scale)
mask = Image.new("L", (SS, SS), 0)
ImageDraw.Draw(mask).rounded_rectangle([0, 0, SS - 1, SS - 1], radius=radius, fill=255)
img.putalpha(mask)

# 白色剪贴板图层（含背景卡片体现“栈”）
white = Image.new("RGBA", (SS, SS), (0, 0, 0, 0))
wd = ImageDraw.Draw(white)

bw, bh = 540, 660
cx, cy = SS // 2, int(SS * 0.54)
body = [cx - bw // 2, cy - bh // 2, cx + bw // 2, cy + bh // 2]

# 后方两张半透明卡片（露出边角，表示多份剪贴记录堆叠）
card_color = (255, 255, 255, 150)
for off in ((-58, -58), (58, -58)):
    rr(wd, [body[0] + off[0], body[1] + off[1], body[2] + off[0], body[2] + off[1]], 70, card_color)

# 主体（不透明白）
rr(wd, body, 70, (255, 255, 255, 255))

# 顶部卡舌（夹子）
clip_w, clip_h = 210, 130
clip = [cx - clip_w // 2, body[1] - clip_h // 2 + int(24 * scale),
        cx + clip_w // 2, body[1] + clip_h // 2 + int(24 * scale)]
rr(wd, clip, 34, (255, 255, 255, 255))

# 文本行
line_color = (185, 198, 255, 255)
line_w = 360
for ly in (cy - 120, cy - 20, cy + 80):
    rr(wd, [cx - line_w // 2, ly, cx + line_w // 2, ly + 30], 15, line_color)

img = Image.alpha_composite(img, white)
img = img.resize((S, S), Image.LANCZOS)
import os
_out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "icon-source.png")
img.save(_out)
print("saved", _out, img.size)
