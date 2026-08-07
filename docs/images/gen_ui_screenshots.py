#!/usr/bin/env python3
"""生成 ClipStack 主界面 / 设置界面的 UI 渲染图（SVG，1:1 还原设计 Token 与布局）。

说明：本环境无法运行 Tauri 原生窗口、也无无头浏览器，故用 SVG 忠实还原应用的实际
界面（颜色取自 tokens.css / format.ts 的 TYPE_META，布局取自 app.css 与各组件）。
SVG 在 GitHub 等 Markdown 渲染器中可内联显示，清晰度不受分辨率影响。

用法：
    python3 gen_ui_screenshots.py
"""

import os

# ---- 设计 Token（与 src/styles/tokens.css、src/lib/format.ts 一致）----
C = {
    "bg": "#ffffff",
    "bg_subtle": "#f7f8fa",
    "bg_hover": "#f1f2f4",
    "border": "#e6e8eb",
    "text_primary": "#1a1d21",
    "text_secondary": "#6b7280",
    "text_tertiary": "#9ca3af",
    "accent": "#059669",
    "accent_soft": "#ecfdf5",
    "t_text": "#059669",
    "t_link": "#2563eb",
    "t_code": "#7c3aed",
    "t_image": "#ea580c",
    "t_file": "#6b7280",
}

FONT = 'font-family="-apple-system, BlinkMacSystemFont, \'PingFang SC\', \'Microsoft YaHei\', system-ui, sans-serif"'

# 图标路径（与 src/components/icons.tsx 一致，viewBox 0 0 24 24，stroke 模式）
PATHS = {
    "text": ["M4 7V5h16v2", "M9 19h6", "M12 5v14"],
    "link": [
        "M10 13a5 5 0 0 0 7 0l3-3a5 5 0 0 0-7-7l-1 1",
        "M14 11a5 5 0 0 0-7 0l-3 3a5 5 0 0 0 7 7l1-1",
    ],
    "code": ["M8 18 2 12l6-6", "M16 6l6 6-6 6"],
    "image": [
        "M3 3h18v18H3z",  # 用 rect 近似 rounded_rect（rx 在 path 不好写，单独处理）
    ],
    "file": [
        "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z",
        "M14 2v6h6",
    ],
    "all": ["rect:3,3,7,7,1", "rect:14,3,7,7,1", "rect:3,14,7,7,1", "rect:14,14,7,7,1"],
    "search": ["M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14z", "M21 21l-4.3-4.3"],
    "gear": [
        "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z",
        "M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z",
    ],
    "trash": [
        "M3 6h18",
        "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2",
        "M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6",
        "M10 11v6M14 11v6",
    ],
    "about": ["M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z", "M12 11v5", "M12 8h.01"],
    "copy": [
        "M9 9h11v11H9z",
        "M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1",
    ],
    "pin": ["M12 17v5", "M9 10.76V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v5.76l2 3.24H7z"],
    "star": ["M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14l-5-4.87 6.91-1.01z"],
}


def icon(name, x, y, size, color, sw=2):
    if name == "image":
        # 圆角画框 + 太阳 + 山脉，忠实还原 TypeIcon image
        inner = [
            '<rect x="3" y="3" width="18" height="18" rx="2"/>',
            '<circle cx="8.5" cy="8.5" r="1.5"/>',
            '<path d="M21 15l-5-5L5 21"/>',
        ]
    else:
        inner = []
        for p in PATHS[name]:
            if p.startswith("rect:"):
                # p 形如 "rect:3,3,7,7,1" → x,y,w,h,r
                parts = p[5:].split(",")
                rx, ry, rw, rh, rr = parts[0], parts[1], parts[2], parts[3], parts[4]
                inner.append(
                    f'<rect x="{rx}" y="{ry}" width="{rw}" height="{rh}" rx="{rr}"/>'
                )
            else:
                inner.append(f'<path d="{p}"/>')
    return (
        f'<svg x="{x}" y="{y}" width="{size}" height="{size}" viewBox="0 0 24 24" '
        f'fill="none" stroke="{color}" stroke-width="{sw}" stroke-linecap="round" '
        f'stroke-linejoin="round">{"".join(inner)}</svg>'
    )


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


# ===================== 主界面 =====================
def build_main():
    W, H = 1000, 600
    s = []
    s.append(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" '
        f'viewBox="0 0 {W} {H}" {FONT}>'
    )
    s.append(f'<rect width="{W}" height="{H}" fill="{C["bg"]}"/>')

    # ---- 侧边栏 ----
    s.append(f'<rect x="0" y="0" width="240" height="{H}" fill="{C["bg_subtle"]}"/>')
    s.append(f'<rect x="239" y="0" width="1" height="{H}" fill="{C["border"]}"/>')

    # 搜索框
    s.append(
        f'<rect x="12" y="14" width="216" height="36" rx="10" fill="{C["bg"]}" '
        f'stroke="{C["border"]}"/>'
    )
    s.append(icon("search", 24, 23, 16, C["text_tertiary"]))
    s.append(
        f'<text x="46" y="37" font-size="13" fill="{C["text_tertiary"]}">搜索剪贴板…</text>'
    )

    nav = [
        ("all", "全部", "12", True, None),
        ("text", "文本", "6", False, C["t_text"]),
        ("link", "链接", "2", False, C["t_link"]),
        ("code", "代码", "1", False, C["t_code"]),
        ("image", "图片", "2", False, C["t_image"]),
        ("file", "文件", "1", False, C["t_file"]),
    ]
    y = 64
    for key, label, count, active, color in nav:
        if active:
            s.append(
                f'<rect x="8" y="{y}" width="224" height="36" rx="10" fill="{C["accent_soft"]}"/>'
            )
        icon_color = C["text_tertiary"] if (key == "all" and active) else (color or C["text_primary"])
        s.append(icon(key, 20, y + 9, 18, icon_color))
        lbl_color = C["accent"] if active else C["text_primary"]
        fw = " font-weight=\"600\"" if active else ""
        s.append(
            f'<text x="46" y="{y + 23}" font-size="14" fill="{lbl_color}"{fw}>{esc(label)}</text>'
        )
        s.append(
            f'<text x="224" y="{y + 23}" font-size="12" fill="{C["text_tertiary"]}" '
            f'text-anchor="end">{count}</text>'
        )
        y += 38

    # 分隔线
    s.append(f'<rect x="8" y="{y}" width="224" height="1" fill="{C["border"]}"/>')
    y += 12
    bottom = [("gear", "设置", C["text_secondary"]),
              ("trash", "回收站", C["text_secondary"]),
              ("about", "关于", C["text_secondary"])]
    for key, label, color in bottom:
        s.append(icon(key, 20, y + 9, 18, color))
        s.append(f'<text x="46" y="{y + 23}" font-size="14" fill="{C["text_primary"]}">{esc(label)}</text>')
        y += 38

    # ---- 中间列表 ----
    LX = 240
    s.append(f'<rect x="{LX}" y="0" width="400" height="{H}" fill="{C["bg"]}"/>')
    s.append(f'<rect x="{LX + 399}" y="0" width="1" height="{H}" fill="{C["border"]}"/>')

    # 工具栏
    s.append(f'<rect x="{LX}" y="0" width="400" height="56" fill="{C["bg"]}"/>')
    s.append(f'<rect x="{LX}" y="55" width="400" height="1" fill="{C["border"]}"/>')
    tabs = [("全部", True), ("今天", False), ("昨天", False), ("本周", False)]
    tx = LX + 16
    for tlabel, tactive in tabs:
        tw = 56
        if tactive:
            s.append(
                f'<rect x="{tx}" y="14" width="{tw}" height="30" rx="6" fill="{C["bg_hover"]}"/>'
            )
        tc = C["text_primary"] if tactive else C["text_secondary"]
        fw = ' font-weight="600"' if tactive else ""
        s.append(
            f'<text x="{tx + tw / 2}" y="34" font-size="13" fill="{tc}" text-anchor="middle"{fw}>{esc(tlabel)}</text>'
        )
        tx += tw + 6

    # 清除全部 + 计数
    s.append(
        f'<text x="{LX + 400 - 130}" y="37" font-size="12" fill="{C["text_tertiary"]}" text-anchor="end">12 条</text>'
    )
    s.append(
        f'<rect x="{LX + 400 - 122}" y="13" width="110" height="30" rx="6" '
        f'fill="{C["bg_subtle"]}" stroke="{C["border"]}"/>'
    )
    s.append(icon("trash", LX + 400 - 116, 19, 14, C["text_secondary"]))
    s.append(
        f'<text x="{LX + 400 - 98}" y="33" font-size="13" fill="{C["text_secondary"]}">清除全部</text>'
    )

    # 列表分组
    s.append(
        f'<text x="{LX + 16}" y="74" font-size="12" font-weight="600" fill="{C["text_tertiary"]}">今天</text>'
    )

    rows = [
        ("code", "fn copy_clipboard() { let cb = Clipboard::new()?; }", "VSCode", C["t_code"], True, None),
        ("link", "https://tauri.app/start/", "Chrome", C["t_link"], False, None),
        ("text", "ClipStack 是一款跨平台剪贴板管理器，让你的剪贴板可回溯、可搜索…", "备忘录", C["t_text"], False, None),
        ("image", "screenshot-2026-08-07.png", "访达", C["t_image"], False, None),
        ("text", "周四 15:00 团队周会，议题：迭代评审与排期", "微信", C["t_text"], False, "pin"),
    ]
    ry = 86
    for key, prev, app, color, selected, badge in rows:
        if selected:
            s.append(
                f'<rect x="{LX + 8}" y="{ry}" width="384" height="48" rx="10" fill="{C["accent_soft"]}"/>'
            )
        # 类型小图标（淡色圆底）
        s.append(
            f'<rect x="{LX + 16}" y="{ry + 8}" width="32" height="32" rx="9" fill="{color}" opacity="0.10"/>'
        )
        s.append(icon(key, LX + 22, ry + 14, 20, color))
        # 预览（截断）
        pv = prev
        if len(pv) > 30:
            pv = pv[:29] + "…"
        s.append(
            f'<text x="{LX + 56}" y="{ry + 21}" font-size="14" fill="{C["text_primary"]}">{esc(pv)}</text>'
        )
        # 来源 / 徽标
        sub = app
        s.append(
            f'<text x="{LX + 56}" y="{ry + 39}" font-size="12" fill="{C["text_tertiary"]}">{esc(sub)}</text>'
        )
        if badge == "pin":
            s.append(icon("pin", LX + 56 + len(sub) * 12 + 6, ry + 27, 13, C["accent"]))
        ry += 52

    # ---- 右侧详情 ----
    DX = 640
    s.append(f'<rect x="{DX}" y="0" width="360" height="{H}" fill="{C["bg_subtle"]}"/>')

    # 头部类型 chip + 来源
    s.append(
        f'<rect x="{DX + 16}" y="24" width="66" height="28" rx="8" fill="{C["t_code"]}" opacity="0.12"/>'
    )
    s.append(icon("code", DX + 22, 30, 16, C["t_code"]))
    s.append(
        f'<text x="{DX + 44}" y="43" font-size="13" font-weight="600" fill="{C["t_code"]}">代码</text>'
    )
    s.append(
        f'<text x="{DX + 344}" y="43" font-size="13" fill="{C["text_secondary"]}" text-anchor="end">VSCode</text>'
    )

    # 预览框（代码）
    s.append(
        f'<rect x="{DX + 16}" y="64" width="328" height="180" rx="14" '
        f'fill="{C["bg"]}" stroke="{C["border"]}"/>'
    )
    code_lines = [
        "fn copy_clipboard() -> Result<()> {",
        "    let mut cb = Clipboard::new()?;",
        "    let text = cb.get_text()?;",
        "    // 命中去重则跳过",
        "    if seen(&text) { return Ok(()); }",
        "    store(&text);",
        "    Ok(())",
        "}",
    ]
    cy = 86
    for line in code_lines:
        s.append(
            f'<text x="{DX + 30}" y="{cy}" font-size="12" fill="{C["text_primary"]}" '
            f'font-family="ui-monospace, Menlo, Consolas, monospace">{esc(line)}</text>'
        )
        cy += 20

    # 元数据
    meta = [("来源", "VSCode"), ("大小", "1.2 KB"), ("时间", "2026-08-07 14:32:10"),
            ("哈希", "a1b2c3d4e5f6")]
    my = 286
    for k, v in meta:
        s.append(
            f'<text x="{DX + 16}" y="{my}" font-size="13" fill="{C["text_tertiary"]}">{esc(k)}</text>'
        )
        if k == "哈希":
            s.append(
                f'<text x="{DX + 344}" y="{my}" text-anchor="end" '
                f'font-family="ui-monospace, Menlo, Consolas, monospace" font-size="11" '
                f'fill="{C["text_primary"]}">{esc(v)}</text>'
            )
        else:
            s.append(
                f'<text x="{DX + 344}" y="{my}" font-size="13" fill="{C["text_primary"]}" '
                f'text-anchor="end">{esc(v)}</text>'
            )
        my += 32

    # 操作按钮
    btns = [("copy", "复制", True, "#ffffff", C["accent"], C["accent"]),
            ("pin", "置顶", False, C["text_primary"], C["bg"], C["border"]),
            ("star", "收藏", False, C["text_primary"], C["bg"], C["border"]),
            ("trash", "删除", False, "#dc2626", C["bg"], "#fca5a5")]
    bx = DX + 16
    by = 540
    for key, label, primary, txtc, bgc, bdc in btns:
        bw = 78 if key == "copy" else 66
        s.append(
            f'<rect x="{bx}" y="{by}" width="{bw}" height="36" rx="10" '
            f'fill="{bgc}" stroke="{bdc}"/>'
        )
        ic = "#ffffff" if primary else txtc
        s.append(icon(key, bx + 10, by + 10, 15, ic))
        s.append(
            f'<text x="{bx + 30}" y="{by + 23}" font-size="13" fill="{txtc}">{esc(label)}</text>'
        )
        bx += bw + 8

    s.append("</svg>")
    return "\n".join(s)


# ===================== 设置界面 =====================
def build_settings():
    W, H = 1000, 760
    s = []
    s.append(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" '
        f'viewBox="0 0 {W} {H}" {FONT}>'
    )
    s.append(f'<rect width="{W}" height="{H}" fill="{C["bg"]}"/>')

    s.append(f'<text x="32" y="40" font-size="18" font-weight="700" fill="{C["text_primary"]}">设置</text>')

    def card(y, h, title):
        s.append(
            f'<rect x="32" y="{y}" width="936" height="{h}" rx="14" '
            f'fill="{C["bg_subtle"]}" stroke="{C["border"]}"/>'
        )
        s.append(
            f'<text x="50" y="{y + 28}" font-size="14" font-weight="600" fill="{C["text_primary"]}">{esc(title)}</text>'
        )

    def row(y, label):
        s.append(
            f'<text x="50" y="{y}" font-size="14" fill="{C["text_primary"]}">{esc(label)}</text>'
        )

    # 卡片1 外观
    card(58, 132, "外观")
    row(110, "主题")
    seg = [("浅色", False), ("深色", False), ("跟随系统", True)]
    sx = 740
    for i, (t, act) in enumerate(seg):
        sw = 88 if t == "跟随系统" else 64
        if act:
            s.append(f'<rect x="{sx}" y="92" width="{sw}" height="30" rx="6" fill="{C["accent"]}"/>')
        else:
            s.append(f'<rect x="{sx}" y="92" width="{sw}" height="30" rx="6" fill="{C["bg"]}" stroke="{C["border"]}"/>')
        tc = "#ffffff" if act else C["text_secondary"]
        s.append(
            f'<text x="{sx + sw / 2}" y="112" font-size="13" fill="{tc}" text-anchor="middle">{esc(t)}</text>'
        )
        sx += sw
        if i < 2:
            s.append(f'<rect x="{sx}" y="92" width="1" height="30" fill="{C["border"]}"/>')
    row(158, "语言")
    s.append(f'<rect x="740" y="140" width="200" height="32" rx="8" fill="{C["bg"]}" stroke="{C["border"]}"/>')
    s.append(f'<text x="754" y="161" font-size="13" fill="{C["text_primary"]}">简体中文</text>')
    s.append(icon("about", 912, 146, 16, C["text_tertiary"]))

    # 卡片2 存储
    card(206, 210, "存储")
    row(258, "历史上限（条）")
    s.append(f'<rect x="740" y="240" width="120" height="34" rx="8" fill="{C["bg"]}" stroke="{C["border"]}"/>')
    s.append(f'<text x="754" y="262" font-size="13" fill="{C["text_primary"]}">1000</text>')
    s.append(
        f'<text x="50" y="292" font-size="13" fill="{C["text_secondary"]}">超出上限的最旧记录会被自动清理；调整将在下次启动时生效。</text>'
    )
    row(336, "托盘菜单历史条数")
    s.append(f'<rect x="740" y="318" width="120" height="34" rx="8" fill="{C["bg"]}" stroke="{C["border"]}"/>')
    s.append(f'<text x="754" y="340" font-size="13" fill="{C["text_primary"]}">30</text>')
    s.append(
        f'<text x="50" y="380" font-size="13" fill="{C["text_secondary"]}">托盘图标菜单中显示的最近历史条数（1–100，默认 30），保存后立即生效。</text>'
    )

    # 卡片3 开机自启
    card(432, 76, "开机自启")
    row(478, "登录后自动启动 ClipStack")
    # 开关（关）
    s.append(f'<rect x="900" y="462" width="44" height="24" rx="12" fill="{C["border"]}"/>')
    s.append(f'<circle cx="915" cy="474" r="9" fill="#ffffff"/>')

    # 卡片4 忽略的应用
    card(524, 210, "忽略的应用")
    s.append(
        f'<text x="50" y="582" font-size="13" fill="{C["text_secondary"]}">被忽略应用的复制内容不会被 ClipStack 捕获。名称以小写应用名匹配（如 safari、终端）。</text>'
    )
    # 添加输入
    s.append(f'<rect x="50" y="596" width="430" height="36" rx="10" fill="{C["bg"]}" stroke="{C["border"]}"/>')
    s.append(f'<text x="64" y="619" font-size="13" fill="{C["text_tertiary"]}">输入应用名后回车添加…</text>')
    s.append(f'<rect x="488" y="596" width="64" height="36" rx="10" fill="{C["accent"]}"/>')
    s.append(f'<text x="520" y="619" font-size="13" fill="#ffffff" text-anchor="middle">添加</text>')
    # 系统选择 + 添加选中
    s.append(f'<rect x="50" y="642" width="430" height="36" rx="10" fill="{C["bg"]}" stroke="{C["border"]}"/>')
    s.append(f'<text x="64" y="665" font-size="13" fill="{C["text_tertiary"]}">从已安装应用选择…</text>')
    s.append(icon("about", 456, 648, 16, C["text_tertiary"]))
    s.append(f'<rect x="488" y="642" width="88" height="36" rx="10" fill="{C["bg"]}" stroke="{C["border"]}"/>')
    s.append(f'<text x="532" y="665" font-size="13" fill="{C["text_primary"]}" text-anchor="middle">添加选中</text>')
    # 标签
    tx = 50
    for name in ["safari", "终端"]:
        tw = 30 + len(name) * 13
        s.append(f'<rect x="{tx}" y="690" width="{tw}" height="30" rx="6" fill="{C["bg"]}" stroke="{C["border"]}"/>')
        s.append(f'<text x="{tx + 12}" y="710" font-size="13" fill="{C["text_primary"]}">{esc(name)}</text>')
        s.append(f'<text x="{tx + tw - 12}" y="710" font-size="16" fill="{C["text_tertiary"]}" text-anchor="end">×</text>')
        tx += tw + 10
    s.append(
        f'<text x="50" y="740" font-size="12" fill="{C["text_tertiary"]}">注：可从「已安装应用」下拉快速选择，或在右侧点击 × 移除已忽略的应用。</text>'
    )

    s.append("</svg>")
    return "\n".join(s)


OUT = os.path.dirname(os.path.abspath(__file__))
with open(os.path.join(OUT, "clipstack-main-ui.svg"), "w", encoding="utf-8") as f:
    f.write(build_main())
with open(os.path.join(OUT, "clipstack-settings-ui.svg"), "w", encoding="utf-8") as f:
    f.write(build_settings())
print("wrote clipstack-main-ui.svg and clipstack-settings-ui.svg")
