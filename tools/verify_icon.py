# -*- coding: utf-8 -*-
"""验证 exe 里到底有没有图标资源：抽出 RT_GROUP_ICON / 关联图标，存成 png 供肉眼确认"""
import ctypes
import os
import sys
from ctypes import wintypes

from PIL import Image, ImageDraw

k32 = ctypes.WinDLL("kernel32", use_last_error=True)
k32.LoadLibraryExW.argtypes = [wintypes.LPCWSTR, wintypes.HANDLE, wintypes.DWORD]
k32.LoadLibraryExW.restype = wintypes.HMODULE
k32.FreeLibrary.argtypes = [wintypes.HMODULE]
k32.FindResourceW.argtypes = [wintypes.HMODULE, wintypes.LPVOID, wintypes.LPVOID]
k32.FindResourceW.restype = wintypes.HANDLE
k32.LoadResource.argtypes = [wintypes.HMODULE, wintypes.HANDLE]
k32.LoadResource.restype = wintypes.HANDLE
k32.LockResource.argtypes = [wintypes.HANDLE]
k32.LockResource.restype = wintypes.LPVOID
k32.SizeofResource.argtypes = [wintypes.HMODULE, wintypes.HANDLE]
k32.SizeofResource.restype = wintypes.DWORD

LOAD_LIBRARY_AS_DATAFILE = 0x2
RT_ICON, RT_GROUP_ICON = 3, 14

exe = sys.argv[1] if len(sys.argv) > 1 else r"target\release\rustexplorer.exe"
exe = os.path.abspath(exe)
hmod = k32.LoadLibraryExW(exe, None, LOAD_LIBRARY_AS_DATAFILE)
if not hmod:
    raise SystemExit("LoadLibraryEx 失败 %d" % ctypes.get_last_error())

grp = k32.FindResourceW(hmod, ctypes.c_void_p(1), ctypes.c_void_p(RT_GROUP_ICON))
print("RT_GROUP_ICON(1):", "有" if grp else "无")
if not grp:
    raise SystemExit(1)
hrsrc = k32.LoadResource(hmod, grp)
size = k32.SizeofResource(hmod, grp)
ptr = k32.LockResource(hrsrc)
data = ctypes.string_at(ptr, size)

import struct
res, typ, cnt = struct.unpack_from("<HHH", data, 0)
print("group icon entries:", cnt)
ids = []
for i in range(cnt):
    off = 6 + i * 14
    w, h, cc, r, planes, bits, nbytes, rid = struct.unpack_from("<BBBBHHIH", data, off)
    ids.append((w or 256, h or 256, rid))

# 统计图标像素，确认不是空壳
tiles = []
for (w, h, rid) in ids:
    r = k32.FindResourceW(hmod, ctypes.c_void_p(rid), ctypes.c_void_p(RT_ICON))
    if not r:
        print("  id", rid, "缺失")
        continue
    d = ctypes.string_at(k32.LockResource(k32.LoadResource(hmod, r)),
                         k32.SizeofResource(hmod, r))
    non_a = sum(1 for i in range(0, len(d), 97) if d[i] > 8)
    print("  id %-3d %3dx%-3d bytes=%d 采样非透明=%d" % (rid, w, h, len(d), non_a))
    tiles.append((w, h, rid))

# 用 ExtractIconEx 拿真图，拼一张
shell32 = ctypes.WinDLL("shell32", use_last_error=True)
user32 = ctypes.WinDLL("user32", use_last_error=True)
shell32.ExtractIconExW.argtypes = [wintypes.LPCWSTR, ctypes.c_int,
                                   ctypes.POINTER(wintypes.HICON),
                                   ctypes.POINTER(wintypes.HICON), wintypes.UINT]
n = shell32.ExtractIconExW(exe, -1, None, None, 0)
print("ExtractIconEx 计数:", n)
large = (wintypes.HICON * 1)()
small = (wintypes.HICON * 1)()
shell32.ExtractIconExW(exe, 0, large, small, 1)

gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)
gdi32.CreateCompatibleDC.argtypes = [wintypes.HDC]
gdi32.CreateCompatibleDC.restype = wintypes.HDC
gdi32.CreateCompatibleBitmap.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int]
gdi32.CreateCompatibleBitmap.restype = wintypes.HBITMAP
gdi32.SelectObject.argtypes = [wintypes.HDC, wintypes.HGDIOBJ]
gdi32.SelectObject.restype = wintypes.HGDIOBJ
gdi32.DeleteObject.argtypes = [wintypes.HGDIOBJ]
gdi32.DeleteDC.argtypes = [wintypes.HDC]
gdi32.PatBlt.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int, ctypes.c_int,
                         ctypes.c_int, wintypes.DWORD]
gdi32.GetDIBits.argtypes = [wintypes.HDC, wintypes.HBITMAP, wintypes.UINT, wintypes.UINT,
                            wintypes.LPVOID, wintypes.LPVOID, wintypes.UINT]
user32.DrawIconEx.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int, wintypes.HICON,
                              ctypes.c_int, ctypes.c_int, wintypes.UINT,
                              wintypes.HANDLE, wintypes.UINT]
user32.ReleaseDC.argtypes = [wintypes.HWND, wintypes.HDC]
user32.GetDC.restype = wintypes.HDC
user32.GetDC.argtypes = [wintypes.HWND]


def hicon_to_image(hicon, size):
    if not hicon:
        return None
    hdc = user32.GetDC(None)
    mem = gdi32.CreateCompatibleDC(hdc)
    bmp = gdi32.CreateCompatibleBitmap(hdc, size, size)
    gdi32.SelectObject(mem, bmp)
    gdi32.PatBlt(mem, 0, 0, size, size, 0x00FF0062)  # BLACKNESS 清底
    user32.DrawIconEx(mem, 0, 0, hicon, size, size, 0, None, 0x0003)  # DI_NORMAL
    class BMI(ctypes.Structure):
        _fields_ = [("biSize", wintypes.DWORD), ("biWidth", wintypes.LONG),
                    ("biHeight", wintypes.LONG), ("biPlanes", wintypes.WORD),
                    ("biBitCount", wintypes.WORD), ("biCompression", wintypes.DWORD),
                    ("biSizeImage", wintypes.DWORD), ("biXPelsPerMeter", wintypes.LONG),
                    ("biYPelsPerMeter", wintypes.LONG), ("biClrUsed", wintypes.DWORD),
                    ("biClrImportant", wintypes.DWORD)]
    bi = BMI()
    bi.biSize = ctypes.sizeof(BMI)
    bi.biWidth = size
    bi.biHeight = -size
    bi.biPlanes = 1
    bi.biBitCount = 32
    buf = ctypes.create_string_buffer(size * size * 4)
    gdi32.GetDIBits(mem, bmp, 0, size, buf, ctypes.byref(bi), 0)
    img = Image.frombuffer("RGBA", (size, size), buf, "raw", "BGRA", 0, 1)
    gdi32.DeleteObject(bmp)
    gdi32.DeleteDC(mem)
    user32.ReleaseDC(None, hdc)
    return img


big = hicon_to_image(large[0], 256)
sml = hicon_to_image(small[0], 32)
sheet = Image.new("RGB", (420, 300), (245, 245, 247))
d = ImageDraw.Draw(sheet)
if big:
    sheet.paste(big, (30, 20), big)
if sml:
    sheet.paste(sml, (320, 120), sml)
d.text((30, 284), "extracted from exe: 256 / 32", fill=(60, 60, 60))
out = os.path.join(os.path.dirname(exe), "icon_extracted.png")
sheet.save(out)
print("->", out)
k32.FreeLibrary(hmod)
