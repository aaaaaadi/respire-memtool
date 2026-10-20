"""生成 memTools 应用图标：icon.png (256) / icon.ico (含 256+32 PNG)。

纯标准库实现：zlib + struct 手写 PNG；ICO 容器内嵌 PNG 数据（Windows Vista+ 支持）。
"""
import struct
import zlib
import os

OUT_DIR = os.path.join(os.path.dirname(__file__), "..", "icons")
OUT_DIR = os.path.abspath(OUT_DIR)
os.makedirs(OUT_DIR, exist_ok=True)


def make_gradient(size):
    """紫→蓝对角渐变圆角方块，带中心光晕。返回 RGBA 字节。"""
    rows = []
    for y in range(size):
        row = bytearray()
        for x in range(size):
            # 圆角遮罩
            r = size * 0.22
            corners = 0
            if x < r and y < r:
                corners = 1
            elif x >= size - r and y < r:
                corners = 2
            elif x < r and y >= size - r:
                corners = 3
            elif x >= size - r and y >= size - r:
                corners = 4
            if corners:
                # 判断是否在圆角内
                if corners == 1:
                    cx, cy = r, r
                elif corners == 2:
                    cx, cy = size - r - 1, r
                elif corners == 3:
                    cx, cy = r, size - r - 1
                else:
                    cx, cy = size - r - 1, size - r - 1
                if (x - cx) ** 2 + (y - cy) ** 2 > r * r:
                    row += bytes((0, 0, 0, 0))
                    continue
            t = (x + y) / (2 * size)
            # 紫色 124,92,255 -> 蓝色 79,157,255
            rr = int(124 + (79 - 124) * t)
            gg = int(92 + (157 - 92) * t)
            bb = int(255)
            # 中心光晕
            dx = x - size / 2
            dy = y - size / 2
            glow = max(0.0, 1.0 - (dx * dx + dy * dy) / (size * size * 0.55))
            rr = min(255, int(rr + 22 * glow))
            gg = min(255, int(gg + 26 * glow))
            bb = min(255, int(bb + 30 * glow))
            row += bytes((rr, gg, bb, 255))
        rows.append(bytes(row))
    return b"".join(rows)


def write_png(path, size, rgba):
    def chunk(typ, data):
        c = struct.pack(">I", len(data)) + typ + data
        c += struct.pack(">I", zlib.crc32(typ + data) & 0xFFFFFFFF)
        return c

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    raw = b""
    for y in range(size):
        raw += b"\x00" + rgba[y * size * 4 : (y + 1) * size * 4]
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", ihdr)
    png += chunk(b"IDAT", zlib.compress(raw, 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


def write_ico(path, entries):
    """entries: list of (size, png_bytes)"""
    count = len(entries)
    header = struct.pack("<HHH", 0, 1, count)
    offset = 6 + 16 * count
    data = b""
    for size, png in entries:
        # 目录项：宽/高 0 表示 256
        w = 0 if size >= 256 else size
        h = 0 if size >= 256 else size
        entry = struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(png), offset)
        data += entry
        offset += len(png)
    payload = b"".join(png for _, png in entries)
    with open(path, "wb") as f:
        f.write(header + data + payload)


def png_bytes(size):
    rgba = make_gradient(size)
    raw = b""
    for y in range(size):
        raw += b"\x00" + rgba[y * size * 4 : (y + 1) * size * 4]
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)

    def chunk(typ, data):
        c = struct.pack(">I", len(data)) + typ + data
        c += struct.pack(">I", zlib.crc32(typ + data) & 0xFFFFFFFF)
        return c

    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(
        b"IDAT", zlib.compress(raw, 9)
    ) + chunk(b"IEND", b"")


if __name__ == "__main__":
    png256 = png_bytes(256)
    png32 = png_bytes(32)
    write_png(os.path.join(OUT_DIR, "icon.png"), 256, make_gradient(256))
    write_ico(os.path.join(OUT_DIR, "icon.ico"), [(256, png256), (32, png32)])
    print("icons written to", OUT_DIR)
