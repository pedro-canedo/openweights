#!/usr/bin/env python3
"""Ícones do OpenWeights.

A marca é um "W" de duas metades separadas por uma fenda no vértice central:
a primeira sólida, a segunda mais clara (pesos de intensidades diferentes) —
a fenda é o "open". Mesma geometria de src/components/OpenWeightsLogo.tsx.

Gera PNGs (32/128/256/512), o .ico da barra de tarefas e o favicon SVG.
Sem dependências externas: rasteriza por distância aos segmentos, com
anti-aliasing por subamostragem.
"""

from __future__ import annotations

import math
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "src-tauri" / "icons"
PUBLIC = ROOT / "public"

BG_TOP = (18, 22, 32)
BG_BOTTOM = (8, 10, 16)
MARK = (248, 249, 252)
# Opacidade da segunda metade do W (igual ao SVG do app).
SECOND_ALPHA = 0.5

# Traços no mesmo sistema de coordenadas do SVG (viewBox 10 14 84 72).
STROKE_A = [(19, 23), (35, 77), (48, 47)]
STROKE_B = [(57, 47), (69, 77), (85, 23)]
STROKE_W = 14.0

VIEW = (10.0, 14.0, 84.0, 72.0)  # x, y, largura, altura


def in_butt_segment(
    px: float, py: float, x1: float, y1: float, x2: float, y2: float, half_w: float
) -> bool:
    """Traço com as duas pontas cortadas em reta (equivale a stroke-linecap
    butt do SVG): exige a projeção dentro do segmento, não uma cápsula."""
    dx, dy = x2 - x1, y2 - y1
    len2 = dx * dx + dy * dy
    if len2 == 0:
        return False
    t = ((px - x1) * dx + (py - y1) * dy) / len2
    if t < 0.0 or t > 1.0:
        return False
    return math.hypot(px - (x1 + t * dx), py - (y1 + t * dy)) <= half_w


def near_polyline(px: float, py: float, pts, half_w: float) -> bool:
    for i in range(len(pts) - 1):
        if in_butt_segment(px, py, *pts[i], *pts[i + 1], half_w):
            return True
    # Preenche a cunha que sobraria nos vértices internos (junção).
    for x, y in pts[1:-1]:
        if math.hypot(px - x, py - y) <= half_w:
            return True
    return False


def rounded_mask(size: int, x: int, y: int) -> bool:
    r = size * 0.22
    lo, hi = r, size - 1 - r
    cx = min(max(x, lo), hi)
    cy = min(max(y, lo), hi)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def to_view(size: int, x: float, y: float) -> tuple[float, float]:
    """Pixel do ícone → coordenada do viewBox, com margem proporcional."""
    pad = size * 0.14
    inner = size - 2 * pad
    vx, vy, vw, vh = VIEW
    scale = max(vw, vh)
    return (
        vx + (x - pad) / inner * scale,
        vy + (y - pad) / inner * scale,
    )


def pixel(size: int, x: int, y: int) -> tuple[int, int, int, int]:
    if not rounded_mask(size, x, y):
        return (0, 0, 0, 0)

    t = y / max(size - 1, 1)
    r = int(BG_TOP[0] * (1 - t) + BG_BOTTOM[0] * t)
    g = int(BG_TOP[1] * (1 - t) + BG_BOTTOM[1] * t)
    b = int(BG_TOP[2] * (1 - t) + BG_BOTTOM[2] * t)

    # Nos tamanhos pequenos o traço precisa engrossar para não sumir.
    boost = 1.25 if size <= 32 else 1.1 if size <= 64 else 1.0
    half = STROKE_W * boost / 2

    samples = (0.17, 0.5, 0.83)
    hits_a = hits_b = 0
    for sx in samples:
        for sy in samples:
            vx, vy = to_view(size, x + sx, y + sy)
            if near_polyline(vx, vy, STROKE_A, half):
                hits_a += 1
            elif near_polyline(vx, vy, STROKE_B, half):
                hits_b += 1

    coverage = (hits_a + hits_b * SECOND_ALPHA) / 9.0
    if coverage > 0:
        a = min(1.0, coverage)
        r = int(r * (1 - a) + MARK[0] * a)
        g = int(g * (1 - a) + MARK[1] * a)
        b = int(b * (1 - a) + MARK[2] * a)
    return (r, g, b, 255)


def make_png(size: int) -> bytes:
    raw = bytearray()
    for y in range(size):
        raw.append(0)
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
        dir_entries += struct.pack("<BBBBHHII", s, s, 0, 0, 1, 32, len(data), offset)
        blobs += data
        offset += len(data)
    return header + dir_entries + blobs


def favicon_svg() -> str:
    a = " ".join(f"{'M' if i == 0 else 'L'}{x} {y}" for i, (x, y) in enumerate(STROKE_A))
    b = " ".join(f"{'M' if i == 0 else 'L'}{x} {y}" for i, (x, y) in enumerate(STROKE_B))
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="10 14 84 72">
  <g fill="none" stroke="#e6e9f0" stroke-width="{STROKE_W:.0f}" stroke-linecap="butt" stroke-linejoin="round">
    <path d="{a}"/>
    <path d="{b}" opacity="{SECOND_ALPHA}"/>
  </g>
</svg>
"""


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    PUBLIC.mkdir(parents=True, exist_ok=True)
    sizes = {32: "32x32.png", 128: "128x128.png", 256: "128x128@2x.png", 512: "icon.png"}
    pngs = {}
    for size, name in sizes.items():
        data = make_png(size)
        (OUT / name).write_bytes(data)
        pngs[size] = data
    (OUT / "icon.ico").write_bytes(make_ico({s: pngs[s] for s in (32, 128, 256)}))
    (PUBLIC / "favicon.svg").write_text(favicon_svg(), encoding="utf-8")
    print(f"ícones do OpenWeights gerados em {OUT}")


if __name__ == "__main__":
    main()
