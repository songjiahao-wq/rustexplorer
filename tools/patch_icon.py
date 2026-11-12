# -*- coding: utf-8 -*-
"""
把 assets/icon.ico 注入 exe 的资源段，让资源管理器里显示的是我们的图标。
不依赖任何 Rust 依赖 / build.rs / windres，纯 ctypes 打 Win32 API：

  BeginUpdateResourceW -> UpdateResourceW(RT_ICON ×N, RT_GROUP_ICON) -> EndUpdateResourceW

用法: python patch_icon.py <exe路径>
注意: 只改 exe 副本；先编译再打，打完不要再跑 cargo（会覆盖掉资源）。
"""
import ctypes
import os
import struct
import sys
from ctypes import wintypes

RT_ICON = 3
RT_GROUP_ICON = 14
LANG_NEUTRAL = 0

HERE = os.path.dirname(os.path.abspath(__file__))
ICO = os.path.join(HERE, "..", "assets", "icon.ico")

k32 = ctypes.WinDLL("kernel32", use_last_error=True)
k32.BeginUpdateResourceW.argtypes = [wintypes.LPCWSTR, wintypes.BOOL]
k32.BeginUpdateResourceW.restype = wintypes.HANDLE
k32.UpdateResourceW.argtypes = [
    wintypes.HANDLE, wintypes.LPVOID, wintypes.LPVOID,
    wintypes.WORD, wintypes.LPVOID, wintypes.DWORD,
]
k32.UpdateResourceW.restype = wintypes.BOOL
k32.EndUpdateResourceW.argtypes = [wintypes.HANDLE, wintypes.BOOL]
k32.EndUpdateResourceW.restype = wintypes.BOOL


def parse_ico(path):
    """返回 [(w, h, colorcount, planes, bitcount, data_bytes), ...]"""
    raw = open(path, "rb").read()
    reserved, typ, count = struct.unpack_from("<HHH", raw, 0)
    assert reserved == 0 and typ == 1, "不是 ICO 文件"
    out = []
    for i in range(count):
        off = 6 + i * 16
        w, h, cc, res, planes, bits, size, data_off = struct.unpack_from("<BBBBHHII", raw, off)
        out.append((w, h, cc, planes, bits, raw[data_off:data_off + size]))
    return out


def build_group(images, base_id):
    # GRPICONDIR: reserved(2) type(2) count(2) + n * GRPICONDIRENTRY(14)
    buf = struct.pack("<HHH", 0, 1, len(images))
    for i, (w, h, cc, planes, bits, data) in enumerate(images):
        buf += struct.pack("<BBBBHHIH", w, h, cc, 0, planes, bits, len(data), base_id + i)
    return buf


def main():
    exe = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        HERE, "..", "target", "release", "rustexplorer.exe")
    exe = os.path.abspath(exe)
    images = parse_ico(ICO)
    base_id = 1

    h = k32.BeginUpdateResourceW(exe, False)
    if not h:
        raise OSError("BeginUpdateResourceW 失败: %d" % ctypes.get_last_error())
    try:
        for i, (_w, _h, _cc, _pl, _bt, data) in enumerate(images):
            ok = k32.UpdateResourceW(h, ctypes.c_void_p(RT_ICON),
                                     ctypes.c_void_p(base_id + i),
                                     LANG_NEUTRAL, data, len(data))
            if not ok:
                raise OSError("UpdateResourceW(RT_ICON %d) 失败: %d" % (base_id + i, ctypes.get_last_error()))
        grp = build_group(images, base_id)
        ok = k32.UpdateResourceW(h, ctypes.c_void_p(RT_GROUP_ICON),
                                 ctypes.c_void_p(1), LANG_NEUTRAL, grp, len(grp))
        if not ok:
            raise OSError("UpdateResourceW(RT_GROUP_ICON) 失败: %d" % ctypes.get_last_error())
        if not k32.EndUpdateResourceW(h, False):
            raise OSError("EndUpdateResourceW 失败: %d" % ctypes.get_last_error())
    except Exception:
        k32.EndUpdateResourceW(h, True)  # 回滚
        raise
    print("patched %s  (%d sizes)" % (exe, len(images)))


if __name__ == "__main__":
    main()
