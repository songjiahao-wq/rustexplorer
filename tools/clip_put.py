# -*- coding: utf-8 -*-
"""用 ctypes 往系统剪贴板写 CF_HDROP，模拟"在 Explorer 里复制了文件"，用来验证本程序能不能读到"""
import ctypes
import sys
from ctypes import wintypes

user32 = ctypes.WinDLL("user32", use_last_error=True)
k32 = ctypes.WinDLL("kernel32", use_last_error=True)

user32.OpenClipboard.argtypes = [wintypes.HWND]
user32.OpenClipboard.restype = wintypes.BOOL
user32.EmptyClipboard.restype = wintypes.BOOL
user32.SetClipboardData.argtypes = [wintypes.UINT, wintypes.HANDLE]
user32.SetClipboardData.restype = wintypes.HANDLE
user32.RegisterClipboardFormatW.argtypes = [wintypes.LPCWSTR]
user32.RegisterClipboardFormatW.restype = wintypes.UINT
k32.GlobalAlloc.argtypes = [wintypes.UINT, ctypes.c_size_t]
k32.GlobalAlloc.restype = wintypes.HGLOBAL
k32.GlobalLock.argtypes = [wintypes.HGLOBAL]
k32.GlobalLock.restype = ctypes.c_void_p
k32.GlobalUnlock.argtypes = [wintypes.HGLOBAL]

GMEM_MOVEABLE = 0x0002
CF_HDROP = 15

paths = sys.argv[1:] or [r"C:\Windows\notepad.exe"]
cut = any(p == "CUT" for p in paths)
paths = [p for p in paths if p != "CUT"]

# DROPFILES: pFiles(4) pt(8) fNC(4) fWide(4) = 20
blob = b""
for p in paths:
    blob += p.encode("utf-16-le") + b"\x00\x00"
blob += b"\x00\x00"
hdr = (20).to_bytes(4, "little") + (0).to_bytes(8, "little") + (0).to_bytes(4, "little") + (1).to_bytes(4, "little")
data = hdr + blob

h = k32.GlobalAlloc(GMEM_MOVEABLE, len(data))
p = k32.GlobalLock(h)
ctypes.memmove(p, data, len(data))
k32.GlobalUnlock(h)

if not user32.OpenClipboard(None):
    print("OpenClipboard 失败")
    raise SystemExit(1)
try:
    user32.EmptyClipboard()
    user32.SetClipboardData(CF_HDROP, h)
    fmt = user32.RegisterClipboardFormatW("Preferred DropEffect")
    h2 = k32.GlobalAlloc(GMEM_MOVEABLE, 4)
    p2 = k32.GlobalLock(h2)
    ctypes.cast(p2, ctypes.POINTER(ctypes.c_uint32))[0] = 2 if cut else 1
    k32.GlobalUnlock(h2)
    user32.SetClipboardData(fmt, h2)
finally:
    user32.CloseClipboard()
print("put:", paths, "cut=", cut)
