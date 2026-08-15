#!/usr/bin/env python3
"""Ícones OpenWeights: O+W entrelaçados (logo original) em squircle escuro."""

from __future__ import annotations

import math
import struct
import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"
PUBLIC = Path(__file__).resolve().parent.parent / "public"

BG_TOP = (18, 22, 32)
BG_BOTTOM = (8, 10, 16)
MARK = (248, 249, 252)

CX, CY = 36.0, 50.0
R_OUT, R_IN = 34.0, 20.0
BAR1 = (46.0, 74.0, 76.0, 18.0)
BAR2 = (72.0, 78.0, 98.0, 30.0)
BAR_W = 12.0
GAP_W = 16.0


def dist_to_seg(px: float, py: float, x1: float, y1: float, x2: float, y2: float) -> float:
    dx, dy = x2 - x1, y2 - y1
    len2 = dx * dx + dy * dy
    if len2 == 0:
        return math.hypot(px - x1, py - y1)
    t = max(0.0, min(1.0, ((px - x1) * dx + (py - y1) * dy) / len2))
    return math.hypot(px - (x1 + t * dx), py - (y1 + t * dy))


def in_capsule(px: float, py: float, bar: tuple[float, float, float, float], w: float) -> bool:
    return dist_to_seg(px, py, *bar) <= w / 2


def in_mark(px: float, py: float, size: int) -> bool:
    boost = 1.25 if size <= 32 else 1.08 if size <= 48 else 1.0
    w = BAR_W * boost
    gap = w + 3.5
    r_in = R_IN / (0.92 + 0.08 * boost)
    d = math.hypot(px - CX, py - CY)
    ring = r_in <= d <= R_OUT and not in_capsule(px, py, BAR1, gap)
    return ring or in_capsule(px, py, BAR1, w) or in_capsule(px, py, BAR2, w)


def rounded_mask(size: int, x: int, y: int) -> bool:
    r = size * 0.22
    lo, hi = r, size - 1 - r
    cx = min(max(x, lo), hi)
    cy = min(max(y, lo), hi)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def to_viewbox(size: int, x: float, y: float) -> tuple[float, float]:
    pad = size * 0.12
    inner = size - 2 * pad
    return ((x - pad) / inner * 100.0, (y - pad) / inner * 100.0)


def pixel(size: int, x: int, y: int) -> tuple[int, int, int, int]:
    if not rounded_mask(size, x, y):
        return (0, 0, 0, 0)
    t = y / max(size - 1, 1)
    r = int(BG_TOP[0] * (1 - t) + BG_BOTTOM[0] * t)
    g = int(BG_TOP[1] * (1 - t) + BG_BOTTOM[1] * t)
    b = int(BG_TOP[2] * (1 - t) + BG_BOTTOM[2] * t)
    samples = (0.2, 0.5, 0.8)
    hits = 0
    for sx in samples:
        for sy in samples:
            vx, vy = to_viewbox(size, x + sx, y + sy)
            if 0 <= vx <= 100 and 0 <= vy <= 100 and in_mark(vx, vy, size):
                hits += 1
    if hits:
        a = hits / 9
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
    return """<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <path fill="#e6e9f0" fill-rule="evenodd" d="M36 16a34 34 0 1 1 0 68 34 34 0 0 1 0-68zm0 14a20 20 0 1 0 0 40 20 20 0 0 0 0-40zM48 78 78 22 68 16 38 72z"/>
  <path fill="#e6e9f0" d="M46 74 76 18 86 24 56 80zM72 78 98 30 88 24 62 72z"/>
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
    print(f"icones OpenWeights gerados em {OUT}")


if __name__ == "__main__":
    main()
