# -*- coding: utf-8 -*-
"""用 ctypes 独立读一遍系统剪贴板的 CF_HDROP，验证 cliptest 写进去的东西别的程序能看见"""
import ctypes
import struct
from ctypes import wintypes

user32 = ctypes.WinDLL("user32", use_last_error=True)
k32 = ctypes.WinDLL("kernel32", use_last_error=True)
shell32 = ctypes.WinDLL("shell32", use_last_error=True)

user32.OpenClipboard.argtypes = [wintypes.HWND]
user32.OpenClipboard.restype = wintypes.BOOL
user32.GetClipboardData.argtypes = [wintypes.UINT]
user32.GetClipboardData.restype = wintypes.HANDLE
user32.RegisterClipboardFormatW.argtypes = [wintypes.LPCWSTR]
user32.RegisterClipboardFormatW.restype = wintypes.UINT
user32.IsClipboardFormatAvailable.argtypes = [wintypes.UINT]
user32.IsClipboardFormatAvailable.restype = wintypes.BOOL
k32.GlobalLock.argtypes = [wintypes.HGLOBAL]
k32.GlobalLock.restype = ctypes.c_void_p
k32.GlobalUnlock.argtypes = [wintypes.HGLOBAL]
shell32.DragQueryFileW.argtypes = [wintypes.HANDLE, wintypes.UINT, wintypes.LPWSTR, wintypes.UINT]
shell32.DragQueryFileW.restype = wintypes.UINT

CF_HDROP = 15

if not user32.IsClipboardFormatAvailable(CF_HDROP):
    print("剪贴板里没有 CF_HDROP")
    raise SystemExit(1)
if not user32.OpenClipboard(None):
    print("OpenClipboard 失败", ctypes.get_last_error())
    raise SystemExit(1)
try:
    h = user32.GetClipboardData(CF_HDROP)
    n = shell32.DragQueryFileW(h, 0xFFFFFFFF, None, 0)
    print("条目数:", n)
    for i in range(n):
        need = shell32.DragQueryFileW(h, i, None, 0)
        buf = ctypes.create_unicode_buffer(need + 1)
        shell32.DragQueryFileW(h, i, buf, need + 1)
        print("  ", buf.value)

    fmt = user32.RegisterClipboardFormatW("Preferred DropEffect")
    h2 = user32.GetClipboardData(fmt)
    if h2:
        p = k32.GlobalLock(h2)
        v = ctypes.cast(p, ctypes.POINTER(ctypes.c_uint32))[0]
        k32.GlobalUnlock(h2)
        print("DropEffect:", v, "(1=复制, 2=剪切)")
finally:
    user32.CloseClipboard()
