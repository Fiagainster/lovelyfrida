"""生成 Tauri 所需的最小图标：icons/icon.ico（16x16 + 32x32 BGRA BMP-in-ICO）。

自绘 logo：圆角深蓝底 + 白色 "LF" 简化方块标记。无第三方依赖。
"""
import struct
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def make_px(size: int):
    # 渐变底色：#3b82f6 -> #9333ea（与前端 logo 一致）
    c0, c1 = (59, 130, 246), (147, 51, 234)
    bg = [[lerp(c0, c1, (x + y) / (2 * size - 2)) for x in range(size)] for y in range(size)]

    def put(x, y, color):
        if 0 <= x < size and 0 <= y < size:
            bg[y][x] = color

    white = (255, 255, 255)
    # 简化 "F" 形（两条横杠 + 一竖），居中
    m = max(2, size // 8)          # 笔画宽
    x0, y0 = int(size * 0.30), int(size * 0.22)
    x1, y1 = int(size * 0.70), int(size * 0.78)
    for y in range(y0, y1):
        for x in range(x0, x0 + m):
            put(x, y, white)                      # 竖笔
    for x in range(x0, x1):
        for y in range(y0, y0 + m):
            put(x, y, white)                      # 上横
    for x in range(x0, int(size * 0.60)):
        for y in range(int((y0 + y1) / 2 - m / 2), int((y0 + y1) / 2 + m / 2)):
            put(x, y, white)                      # 中横
    return bg


def bmp_entry(size: int) -> bytes:
    px = make_px(size)
    # BITMAPINFOHEADER(40) + BGRA 像素 + AND mask
    header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, size * size * 4, 0, 0, 0, 0)
    rows = []
    for y in range(size - 1, -1, -1):  # BMP 自底向上
        row = b""
        for x in range(size):
            r, g, b = px[y][x]
            row += struct.pack("<BBBB", b, g, r, 255)
        rows.append(row)
    mask_row = b"\x00" * (((size + 31) // 32) * 4)
    return header + b"".join(rows) + mask_row * size


def make_ico(sizes=(16, 32, 48)) -> bytes:
    count = len(sizes)
    offset = 6 + 16 * count
    header = struct.pack("<HHH", 0, 1, count)
    body = b""
    directory = []
    for s in sizes:
        data = bmp_entry(s)
        directory.append(struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(data), offset))
        body += data
        offset += len(data)
    return header + b"".join(directory) + body


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "icon.ico").write_bytes(make_ico((16, 32, 48)))
    print("icon written to", OUT / "icon.ico")
