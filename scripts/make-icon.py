"""Writes assets/mocli.ico: the clipboard of the tray icon at every size
Windows asks for. Run from the repository root; needs only the standard library.
"""

import struct
import zlib
from pathlib import Path

SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)
BOARD = (255, 122, 26, 255)
CLIP = (200, 200, 200, 255)
LINE = (255, 255, 255, 255)
CLEAR = (0, 0, 0, 0)


def color_at(x: float, y: float):
    """The color at a point of the 32 by 32 design."""
    if 11 <= x < 21 and 2 <= y < 8:
        return CLIP
    if 9 <= x < 23 and any(top <= y < top + 2 for top in (13, 18, 23)):
        return LINE
    # A board with rounded corners.
    if 5 <= x < 27 and 5 <= y < 30:
        radius = 3
        cx = min(max(x, 5 + radius), 27 - radius)
        cy = min(max(y, 5 + radius), 30 - radius)
        if (x - cx) ** 2 + (y - cy) ** 2 <= radius**2:
            return BOARD
    return CLEAR


def render(size: int, samples: int = 4) -> bytes:
    """RGBA rows, smoothed by averaging several samples per pixel."""
    rows = bytearray()
    for py in range(size):
        rows.append(0)  # PNG row filter: none
        for px in range(size):
            total = [0, 0, 0, 0]
            for sy in range(samples):
                for sx in range(samples):
                    x = (px + (sx + 0.5) / samples) * 32 / size
                    y = (py + (sy + 0.5) / samples) * 32 / size
                    red, green, blue, alpha = color_at(x, y)
                    total[0] += red * alpha
                    total[1] += green * alpha
                    total[2] += blue * alpha
                    total[3] += alpha
            alpha = total[3] / samples**2
            if total[3]:
                rows.extend(round(c / total[3]) for c in total[:3])
            else:
                rows.extend((0, 0, 0))
            rows.append(round(alpha))
    return bytes(rows)


def png(size: int) -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(render(size), 9))
        + chunk(b"IEND", b"")
    )


def main() -> None:
    images = [png(size) for size in SIZES]
    directory = struct.pack("<HHH", 0, 1, len(images))
    offset = len(directory) + 16 * len(images)
    for size, data in zip(SIZES, images):
        edge = 0 if size == 256 else size
        directory += struct.pack("<BBBBHHII", edge, edge, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    target = Path("assets/mocli.ico")
    target.parent.mkdir(exist_ok=True)
    target.write_bytes(directory + b"".join(images))
    print(f"wrote {target} ({target.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
