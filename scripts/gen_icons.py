#!/usr/bin/env python3
"""Gera ícones placeholder (PNG + ICO) sem dependências externas.

Quadrado arredondado violeta com uma "chama" clara — placeholder até a
identidade visual real (M5). PNGs gerados: 32, 128, 256 (@2x), 512.
O .ico embute os PNGs (formato PNG-in-ICO, suportado desde o Vista).
"""

import struct
import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"

ACCENT = (124, 92, 255)  # #7c5cff
DARK = (23, 18, 48)


def rounded_mask(size: int, x: int, y: int) -> bool:
    r = size * 0.22
    lo, hi = r, size - 1 - r
    cx = min(max(x, lo), hi)
    cy = min(max(y, lo), hi)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def pixel(size: int, x: int, y: int):
    if not rounded_mask(size, x, y):
        return (0, 0, 0, 0)
    # Gradiente diagonal do accent para um tom mais escuro.
    t = (x + y) / (2 * size)
    r = int(ACCENT[0] * (1 - t) + DARK[0] * t)
    g = int(ACCENT[1] * (1 - t) + DARK[1] * t)
    b = int(ACCENT[2] * (1 - t) + DARK[2] * t)
    # "Chama" clara: círculo deslocado ao alto.
    dx, dy = x - size * 0.5, y - size * 0.42
    if dx * dx + dy * dy <= (size * 0.2) ** 2:
        r = min(255, r + 90)
        g = min(255, g + 90)
        b = min(255, b + 60)
    return (r, g, b, 255)


def make_png(size: int) -> bytes:
    raw = bytearray()
    for y in range(size):
        raw.append(0)  # filtro none
        for x in range(size):
            raw.extend(pixel(size, x, y))

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def make_ico(pngs: dict[int, bytes]) -> bytes:
    entries = sorted(pngs.items())
    header = struct.pack("<HHH", 0, 1, len(entries))
    dir_entries = b""
    blobs = b""
    offset = 6 + 16 * len(entries)
    for size, data in entries:
        s = 0 if size >= 256 else size
        dir_entries += struct.pack(
            "<BBBBHHII", s, s, 0, 0, 1, 32, len(data), offset
        )
        blobs += data
        offset += len(data)
    return header + dir_entries + blobs


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    sizes = {32: "32x32.png", 128: "128x128.png", 256: "128x128@2x.png", 512: "icon.png"}
    pngs = {}
    for size, name in sizes.items():
        data = make_png(size)
        (OUT / name).write_bytes(data)
        pngs[size] = data
    (OUT / "icon.ico").write_bytes(make_ico({s: pngs[s] for s in (32, 128, 256)}))
    print(f"ícones gerados em {OUT}")


if __name__ == "__main__":
    main()
