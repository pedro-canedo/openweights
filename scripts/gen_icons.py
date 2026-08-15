#!/usr/bin/env python3
"""Gera os ícones do Rift (PNG + ICO) sem dependências externas.

Identidade visual: fundo escuro (quadrado arredondado) com a FENDA (rift)
branca vertical — o mesmo traço que substitui o "I" no wordmark R|FT.
PNGs: 32, 128, 256 (@2x), 512. O .ico embute os PNGs (PNG-in-ICO).
"""

import struct
import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"

BG_TOP = (16, 20, 29)     # #10141d
BG_BOTTOM = (5, 7, 11)    # #05070b
CRACK = (245, 247, 250)   # branco levemente frio

# Linha central da fenda: (x, y, largura), tudo em frações do tamanho.
# Mesmo desenho do RiftMark em src/components/RiftLogo.tsx.
SPINE = [
    (0.500, 0.14, 0.012),
    (0.455, 0.30, 0.030),
    (0.535, 0.44, 0.042),
    (0.460, 0.60, 0.046),
    (0.525, 0.74, 0.032),
    (0.480, 0.86, 0.012),
]


def crack_polygon(size: int) -> list[tuple[float, float]]:
    # Fenda mais grossa nos tamanhos pequenos para não sumir.
    boost = 1.6 if size <= 48 else 1.2 if size <= 128 else 1.0
    left = [((cx - w * boost / 2) * size, cy * size) for cx, cy, w in SPINE]
    right = [((cx + w * boost / 2) * size, cy * size) for cx, cy, w in reversed(SPINE)]
    return left + right


def point_in_poly(x: float, y: float, poly: list[tuple[float, float]]) -> bool:
    inside = False
    j = len(poly) - 1
    for i in range(len(poly)):
        xi, yi = poly[i]
        xj, yj = poly[j]
        if (yi > y) != (yj > y) and x < (xj - xi) * (y - yi) / (yj - yi) + xi:
            inside = not inside
        j = i
    return inside


def rounded_mask(size: int, x: int, y: int) -> bool:
    r = size * 0.22
    lo, hi = r, size - 1 - r
    cx = min(max(x, lo), hi)
    cy = min(max(y, lo), hi)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def pixel(size: int, x: int, y: int, poly, bbox) -> tuple[int, int, int, int]:
    if not rounded_mask(size, x, y):
        return (0, 0, 0, 0)
    # Gradiente vertical sutil.
    t = y / size
    r = int(BG_TOP[0] * (1 - t) + BG_BOTTOM[0] * t)
    g = int(BG_TOP[1] * (1 - t) + BG_BOTTOM[1] * t)
    b = int(BG_TOP[2] * (1 - t) + BG_BOTTOM[2] * t)
    # Cobertura da fenda com 3x3 subamostras (anti-aliasing); o teste de
    # polígono só roda dentro do bounding box (economia de ~90% do tempo).
    hits = 0
    x0, y0, x1, y1 = bbox
    if x0 - 1 <= x <= x1 + 1 and y0 - 1 <= y <= y1 + 1:
        for sx in (0.17, 0.5, 0.83):
            for sy in (0.17, 0.5, 0.83):
                if point_in_poly(x + sx, y + sy, poly):
                    hits += 1
    if hits:
        a = hits / 9
        r = int(r * (1 - a) + CRACK[0] * a)
        g = int(g * (1 - a) + CRACK[1] * a)
        b = int(b * (1 - a) + CRACK[2] * a)
    return (r, g, b, 255)


def make_png(size: int) -> bytes:
    poly = crack_polygon(size)
    xs = [p[0] for p in poly]
    ys = [p[1] for p in poly]
    bbox = (min(xs), min(ys), max(xs), max(ys))
    raw = bytearray()
    for y in range(size):
        raw.append(0)  # filtro none
        for x in range(size):
            raw.extend(pixel(size, x, y, poly, bbox))

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
        dir_entries += struct.pack("<BBBBHHII", s, s, 0, 0, 1, 32, len(data), offset)
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
    print(f"ícones do Rift gerados em {OUT}")


if __name__ == "__main__":
    main()
