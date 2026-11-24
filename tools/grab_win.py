"""直接抓窗口内容（PrintWindow + PW_RENDERFULLCONTENT）。

前台被别的程序占用时，ImageGrab.grab(bbox) 会拍到覆盖在上面的窗口；
PrintWindow 让窗口自己把内容画到内存 DC，不依赖可见性与前台。
用法: python tools/grab_win.py <exe> <dir> <out.jpg>
"""
import ctypes
import os
import subprocess
import sys
import time
from ctypes import wintypes

import win32gui
import win32ui
import win32con
from PIL import Image

user32 = ctypes.windll.user32


def find_windows(pid):
    out = []

    def cb(h, _):
        if win32gui.IsWindowVisible(h):
            wpid = wintypes.DWORD()
            user32.GetWindowThreadProcessId(h, ctypes.byref(wpid))
            if wpid.value == pid and win32gui.GetWindowText(h):
                out.append(h)
        return True

    win32gui.EnumWindows(cb, None)
    return out


def grab(hwnd, path):
    l, t, r, b = win32gui.GetWindowRect(hwnd)
    w, hgt = r - l, b - t
    hdc = win32gui.GetWindowDC(hwnd)
    src = win32ui.CreateDCFromHandle(hdc)
    mem = src.CreateCompatibleDC()
    bmp = win32ui.CreateBitmap()
    bmp.CreateCompatibleBitmap(src, w, hgt)
    mem.SelectObject(bmp)
    # 2 = PW_RENDERFULLCONTENT（Win 8.1+，GPU 合成的窗口也能拿到内容）
    ok = user32.PrintWindow(hwnd, mem.GetSafeHdc(), 2)
    info = bmp.GetInfo()
    data = bmp.GetBitmapBits(True)
    img = Image.frombuffer("RGB", (info["bmWidth"], info["bmHeight"]), data, "raw", "BGRX", 0, 1)
    img.save(path, quality=92)
    mem.DeleteDC()
    src.DeleteDC()
    win32gui.ReleaseDC(hwnd, hdc)
    win32gui.DeleteObject(bmp.GetHandle())
    return ok, (w, hgt)


if __name__ == "__main__":
    exe, d, out = sys.argv[1], sys.argv[2], sys.argv[3]
    p = subprocess.Popen([exe, d])
    hw = []
    for _ in range(40):
        hw = find_windows(p.pid)
        if hw:
            break
        time.sleep(0.25)
    if not hw:
        print("no window")
        p.terminate()
        sys.exit(1)
    time.sleep(2.0)
    ok, size = grab(hw[0], out)
    print("printwindow", ok, size)
    p.terminate()
