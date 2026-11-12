# -*- coding: utf-8 -*-
"""
RustExplorer 图标生成器（3 个方案）

用法:
  python gen_icon.py                -> 只出对比图 assets/icon_preview.png，三方案并排
  python gen_icon.py a|b|c          -> 额外把该方案写成 assets/icon.png + assets/icon.ico

方案:
  a  蓝底白文件夹（Win11 蓝 squircle，白文件夹，顶边两个 tab 凸起）
  b  白底蓝文件夹（浅色主题同款：近白底 + Win11 蓝文件夹）
  c  蓝底双卡片（两张叠起来的圆角卡片，强调"多窗口"）
"""
import os
import sys
from PIL import Image, ImageDraw, ImageFilter

HERE = os.path.dirname(os.path.abspath(__file__))
ASSETS = os.path.join(HERE, "..", "assets")
os.makedirs(ASSETS, exist_ok=True)

S = 4
N = 1024 * S
R_SQUIRCLE = int(N * 0.225)

Z = None  # 每个方案自己填 (H,W)


def lerp(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def px(v, n=N):
    return int(v * n / 1024.0)


def gradient_squircle(top, bot, radius=R_SQUIRCLE, n=N):
    bg = Image.new("RGB", (n, n))
    d = ImageDraw.Draw(bg)
    for y in range(n):
        t = (y / (n - 1)) ** 0.92
        d.line([(0, y), (n, y)], fill=lerp(top, bot, t))
    mask = Image.new("L", (n, n), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, n - 1, n - 1], radius=radius, fill=255)
    out = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    out.paste(bg, (0, 0), mask)

    # 内侧顶部高光
    hl = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    hd = ImageDraw.Draw(hl)
    inset = int(n * 0.012)
    hd.rounded_rectangle([inset, inset, n - 1 - inset, n - 1],
                         radius=radius - int(n * 0.01),
                         outline=(255, 255, 255, 50), width=max(2, int(n * 0.006)))
    hl = hl.filter(ImageFilter.GaussianBlur(n * 0.004))
    return Image.alpha_composite(out, hl)


def _folder(lay, back_rgb, front_rgb, line_rgb, width_scale=1.0):
    """在 lay 上画一个文件夹（back 后片 + tab 凸起 + 投影 + front 前片 + 条纹）"""
    dr = ImageDraw.Draw(lay)

    def X(v):
        # 以 1024 设计稿为基准，左右按 width_scale 收一点
        c = 512.0
        return px(c + (v - c) * width_scale)

    # 投影
    sh = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    sd = ImageDraw.Draw(sh)
    sd.rounded_rectangle([X(160), px(470), X(866), px(812)], radius=px(50), fill=(4, 34, 80, 120))
    sh = sh.filter(ImageFilter.GaussianBlur(px(28)))
    lay.alpha_composite(sh) if False else None
    lay2 = Image.alpha_composite(lay, sh)
    lay.paste(lay2, (0, 0))
    dr = ImageDraw.Draw(lay)

    # 后片 + 两个 tab
    dr.rounded_rectangle([X(168), px(324), X(842), px(760)], radius=px(42), fill=back_rgb)
    dr.rounded_rectangle([X(168), px(236), X(470), px(392)], radius=px(34), fill=back_rgb)
    dr.rounded_rectangle([X(488), px(276), X(706), px(392)], radius=px(34), fill=back_rgb)

    # 前片
    dr.rounded_rectangle([X(146), px(418), X(878), px(800)], radius=px(54), fill=front_rgb)
    # 盖子分隔
    top_strip = tuple(min(255, int(c * 0.90 + 255 * 0.10)) for c in line_rgb[:3]) + (255,)
    dr.rounded_rectangle([X(146), px(418), X(878), px(446)], radius=px(26), fill=top_strip)
    # 三条条目线
    for y, w, a in [(px(508), px(320), 70), (px(586), px(430), 50), (px(664), px(248), 34)]:
        dr.rounded_rectangle([X(226), y, X(226) + w, y + px(28)], radius=px(14),
                             fill=line_rgb[:3] + (a,))


def variant_a():
    """蓝底 + 白文件夹"""
    icon = gradient_squircle((77, 163, 255), (0, 84, 200))
    lay = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    _folder(lay, (205, 226, 252, 255), (255, 255, 255, 255), (0, 84, 200, 255))
    return Image.alpha_composite(icon, lay)


def variant_b():
    """近白底 + Win11 蓝文件夹（浅色主题同款）"""
    icon = gradient_squircle((255, 255, 255), (238, 241, 246))
    # 外描边：浅灰，让白底在浅色背景上也有边界
    edge = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    ImageDraw.Draw(edge).rounded_rectangle(
        [px(2), px(2), N - px(2), N - px(2)], radius=R_SQUIRCLE,
        outline=(180, 190, 205, 255), width=px(6))
    icon = Image.alpha_composite(icon, edge)

    lay = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    _folder(lay, (150, 197, 250, 255), (0, 102, 214, 255), (255, 255, 255, 255))
    return Image.alpha_composite(icon, lay)


def variant_c():
    """蓝底 + 两张叠起来的卡片（多窗口）"""
    icon = gradient_squircle((77, 163, 255), (0, 84, 200))
    lay = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    dr = ImageDraw.Draw(lay)
    # 后卡：淡蓝，向右上偏移
    dr.rounded_rectangle([px(300), px(196), px(892), px(604)], radius=px(52),
                         fill=(214, 232, 255, 255))
    # 前卡投影
    sh = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    ImageDraw.Draw(sh).rounded_rectangle([px(120), px(300), px(760), px(830)], radius=px(56),
                                        fill=(4, 34, 80, 120))
    sh = sh.filter(ImageFilter.GaussianBlur(px(26)))
    lay = Image.alpha_composite(lay, sh)
    dr = ImageDraw.Draw(lay)
    # 前卡：白
    dr.rounded_rectangle([px(120), px(286), px(748), px(816)], radius=px(56),
                         fill=(255, 255, 255, 255))
    # 前卡顶部标签条（三个小点 = tab 栏）
    dr.rounded_rectangle([px(120), px(286), px(748), px(392)], radius=px(56),
                         fill=(236, 244, 255, 255))
    for i, cx in enumerate((200, 268, 336)):
        dr.rounded_rectangle([px(cx), px(322), px(cx + 48), px(356)], radius=px(17),
                             fill=(0, 102, 214, 255 if i == 0 else 90))
    # 内容行
    for y, w, a in [(px(470), px(420), 70), (px(556), px(500), 48), (px(642), px(300), 32)]:
        dr.rounded_rectangle([px(196), y, px(196) + w, y + px(30)], radius=px(15),
                             fill=(0, 84, 200, a))
    return Image.alpha_composite(icon, lay)


VARIANTS = {"a": variant_a, "b": variant_b, "c": variant_c}
NAMES = {"a": "A 蓝底白文件夹", "b": "B 白底蓝文件夹", "c": "C 蓝底双卡片"}


def save_all(master):
    m = master.resize((1024, 1024), Image.LANCZOS)
    m.save(os.path.join(ASSETS, "icon.png"), optimize=True)
    m.save(os.path.join(ASSETS, "icon.ico"), format="ICO",
           sizes=[(s, s) for s in (16, 20, 24, 32, 40, 48, 64, 128, 256)])
    # 给 egui 运行时用：256x256 未预乘 RGBA 裸字节（include_bytes! 直接吃）
    small = master.resize((256, 256), Image.LANCZOS)
    with open(os.path.join(ASSETS, "icon_256.rgba"), "wb") as f:
        f.write(small.tobytes("raw", "RGBA"))


def main():
    which = sys.argv[1].lower() if len(sys.argv) > 1 else None
    masters = {k: f() for k, f in VARIANTS.items()}

    # 对比图：三行，每行 [256 / 128 / 64 / 48 / 32 / 16]，浅底
    pad, gap = 26, 26
    row_h = 300
    W = 1100
    sheet = Image.new("RGB", (W, pad + row_h * 3 + pad), (250, 250, 250))
    d = ImageDraw.Draw(sheet)
    y = pad
    for k in ("a", "b", "c"):
        m = masters[k]
        d.text((pad, y - 16), NAMES[k], fill=(40, 44, 52))
        x = pad
        yy = y + 16
        for s in (256, 128, 64, 48, 32, 16):
            im = m.resize((s, s), Image.LANCZOS)
            sheet.paste(im, (x, yy + (256 - s) // 2), im)
            x += s + gap
        y += row_h
    sheet.save(os.path.join(ASSETS, "icon_preview.png"), optimize=True)

    if which in masters:
        save_all(masters[which])
        print("saved variant", which, os.path.getsize(os.path.join(ASSETS, "icon.ico")))
    print("preview ->", os.path.join(ASSETS, "icon_preview.png"))


if __name__ == "__main__":
    main()
