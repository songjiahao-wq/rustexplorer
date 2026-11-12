# -*- coding: utf-8 -*-
import importlib.util
import os
import sys

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("g", os.path.join(HERE, "gen_icon.py"))
g = importlib.util.module_from_spec(spec)
spec.loader.exec_module(g)

for k in sys.argv[1:] or ["b", "c"]:
    im = g.VARIANTS[k]().resize((32, 32), Image.LANCZOS)
    print("=== variant {} 32x32 ===".format(k))
    for y in range(32):
        row = ""
        for x in range(32):
            r, gg, b, a = im.getpixel((x, y))
            if a < 40:
                row += " "
                continue
            lum = (r + gg + b) / 3
            row += "#" if lum > 235 else "+" if lum > 190 else "-" if lum > 140 else "." if lum > 80 else ":"
        print(row)
