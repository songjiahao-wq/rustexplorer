# -*- coding: utf-8 -*-
"""无视觉通道时的 UI 体检：主色分布 / 结构行 / 分栏位置"""
import collections
import sys

from PIL import Image

p = sys.argv[1]
im = Image.open(p).convert("RGB")
w, h = im.size
px = im.load()
print("size", w, h)

cnt = collections.Counter()
for y in range(0, h, 2):
    for x in range(0, w, 2):
        cnt[px[x, y]] += 1
print("top colors:")
for c, n in cnt.most_common(6):
    print("   ", c, n)

# 逐行平均亮度，找面板边界（亮度的跳变列）
rows = []
for y in range(h):
    s = 0
    for x in range(0, w, 6):
        r, g, b = px[x, y]
        s += (r + g + b) / 3
    rows.append(s / len(range(0, w, 6)))
edges = [y for y in range(1, h) if abs(rows[y] - rows[y - 1]) > 6]
print("horizontal edges:", edges[:12])

# 逐列亮度跳变（侧栏/内容分界）
cols = []
for x in range(w):
    s = 0
    for y in range(0, h, 6):
        r, g, b = px[x, y]
        s += (r + g + b) / 3
    cols.append(s / len(range(0, h, 6)))
vedges = [x for x in range(1, w) if abs(cols[x] - cols[x - 1]) > 5]
print("vertical edges:", vedges[:12])

# 蓝色系像素（选中/强调色）统计，确认 accent 还是 Win11 蓝
blue = sum(1 for c, n in cnt.items() if c[2] > c[0] + 20 and c[2] > 150)
cyan = sum(1 for c, n in cnt.items() if c[2] > 250 and c[1] > 250 and c[0] < 235)
print("bluish colors:", blue, " cyanish:", cyan)
