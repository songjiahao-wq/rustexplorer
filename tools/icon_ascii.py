# -*- coding: utf-8 -*-
"""把图标渲染成 ASCII 点阵，方便无视觉通道时检查形状可读性"""
import sys
from PIL import Image

p = sys.argv[1] if len(sys.argv) > 1 else r"D:\AI_project\workbuddy\projects\rustexplorer\assets\icon.png"
im = Image.open(p).convert("RGBA")
for s in (48, 32, 16):
    t = im.resize((s, s), Image.LANCZOS)
    print("=== {}x{} ===".format(s, s))
    for y in range(s):
        row = ""
        for x in range(s):
            r, g, b, a = t.getpixel((x, y))
            if a < 40:
                row += " "
                continue
            lum = (r + g + b) / 3
            if lum > 235:
                row += "#"
            elif lum > 190:
                row += "+"
            elif lum > 140:
                row += "-"
            elif lum > 80:
                row += "."
            else:
                row += ":"
        print(row)
