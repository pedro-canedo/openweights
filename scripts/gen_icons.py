#!/usr/bin/env python3
"""Ícones do OpenWeights.

A marca é um monograma O+W: um anel ABERTO embaixo — o "open", o modelo que
dá para enxergar por dentro — cuja perna esquerda desce e vira o primeiro
traço de um "W". O braço direito do W sobe alto, saindo do anel. Cortando os
dois na diagonal, um raio azul: o peso que atravessa a máquina.

Mesma geometria de src/components/OpenWeightsLogo.tsx — quem mexer aqui
precisa mexer lá.

Gera os PNGs do app (32/128/256/512), o .ico da barra de tarefas, o favicon
SVG e os arquivos de marca em brand/. Sem dependências externas: rasteriza
por distância às formas, com anti-aliasing por subamostragem.
"""

from __future__ import annotations

import math
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "src-tauri" / "icons"
PUBLIC = ROOT / "public"
BRAND = ROOT / "brand"

BG_TOP = (26, 27, 31)
BG_BOTTOM = (10, 11, 14)
# Prata: claro em cima, escurecendo embaixo — é o que dá o volume sem
# precisar de sombra.
MARK_TOP = (252, 253, 255)
MARK_BOTTOM = (146, 154, 172)
BOLT_CORE = (120, 120, 255)
BOLT_GLOW = (40, 40, 220)

# ------------------------------------------------------------- geometria ---
# Sistema de coordenadas do viewBox 0 0 100 100.

VIEW = (0.0, 0.0, 100.0, 100.0)

RING_CX, RING_CY, RING_R = 47.5, 40.0, 16.7
#: Ângulos (graus, y para baixo) do arco desenhado: começa na perna esquerda
#: (7h30), sobe pela esquerda, passa pelo topo e desce até a perna direita
#: (4h30). A fenda larga embaixo é por onde o W passa — e é o "open".
RING_FROM, RING_TO = 137.0, 400.0
RING_W = 7.6

#: O W começa exatamente na ponta da perna esquerda do anel: a perna É o
#: primeiro traço. O último braço sobe alto, saindo por fora do anel.
W_POINTS = [(29.5, 53.0), (42.0, 79.0), (51.0, 55.0), (60.0, 79.0), (80.0, 40.0)]
W_W = 7.6

#: O raio: uma agulha quase horizontal que atravessa a marca inteira, das
#: pontas finas ao meio grosso, passando pela fenda do anel.
BOLT_A = (12.0, 76.0)
BOLT_B = (89.0, 39.0)
BOLT_HALF = 1.5
#: Brilho em volta do raio (em unidades do viewBox).
BOLT_GLOW_R = 2.6


def ring_point(deg: float) -> tuple[float, float]:
    a = math.radians(deg)
    return (RING_CX + RING_R * math.cos(a), RING_CY + RING_R * math.sin(a))


def in_arc(px: float, py: float, half_w: float) -> bool:
    d = math.hypot(px - RING_CX, py - RING_CY)
    if abs(d - RING_R) > half_w:
        return False
    ang = math.degrees(math.atan2(py - RING_CY, px - RING_CX)) % 360.0
    # O arco cruza 360°, então comparamos na volta desenrolada.
    if ang < RING_FROM:
        ang += 360.0
    return RING_FROM <= ang <= RING_TO


def in_butt_segment(
    px: float, py: float, x1: float, y1: float, x2: float, y2: float, half_w: float
) -> bool:
    """Traço com as pontas cortadas em reta (o `stroke-linecap: butt` do SVG):
    exige a projeção dentro do segmento, não uma cápsula."""
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


def bolt_distance(px: float, py: float) -> tuple[float, float]:
    """Distância à linha do raio e a posição ao longo dela (0..1)."""
    x1, y1 = BOLT_A
    x2, y2 = BOLT_B
    dx, dy = x2 - x1, y2 - y1
    len2 = dx * dx + dy * dy
    t = max(0.0, min(1.0, ((px - x1) * dx + (py - y1) * dy) / len2))
    return math.hypot(px - (x1 + t * dx), py - (y1 + t * dy)), t


def in_bolt(px: float, py: float, boost: float) -> bool:
    d, t = bolt_distance(px, py)
    # Afina para as pontas: é o que faz parecer uma agulha, e não uma barra.
    half = BOLT_HALF * boost * (1.0 - abs(2.0 * t - 1.0)) ** 0.45
    return d <= half


# -------------------------------------------------------------- rasteriza ---


def rounded_mask(size: int, x: int, y: int) -> bool:
    r = size * 0.22
    lo, hi = r, size - 1 - r
    cx = min(max(x, lo), hi)
    cy = min(max(y, lo), hi)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def to_view(size: int, x: float, y: float) -> tuple[float, float]:
    """Pixel do ícone → coordenada do viewBox, com margem proporcional."""
    # A margem já está nas coordenadas (medidas da marca de referência).
    pad = 0.0
    inner = size - 2 * pad
    vx, vy, vw, vh = VIEW
    scale = max(vw, vh)
    return (vx + (x - pad) / inner * scale, vy + (y - pad) / inner * scale)


def mix(a: tuple[int, int, int], b: tuple[int, int, int], t: float):
    return tuple(int(a[i] * (1 - t) + b[i] * t) for i in range(3))


def pixel(size: int, x: int, y: int) -> tuple[int, int, int, int]:
    if not rounded_mask(size, x, y):
        return (0, 0, 0, 0)

    t = y / max(size - 1, 1)
    r, g, b = mix(BG_TOP, BG_BOTTOM, t)

    # Engrossar demais fecha o furo do anel e a marca vira um borrão: nos
    # tamanhos pequenos o que salva a leitura é o contraste, não a espessura.
    boost = 1.1 if size <= 32 else 1.04 if size <= 64 else 1.0
    half_ring = RING_W * boost / 2
    half_w = W_W * boost / 2

    samples = (0.17, 0.5, 0.83)
    prata = 0
    raio = 0
    brilho = 0.0
    for sx in samples:
        for sy in samples:
            vx, vy = to_view(size, x + sx, y + sy)
            if in_bolt(vx, vy, boost):
                raio += 1
            elif in_arc(vx, vy, half_ring) or near_polyline(vx, vy, W_POINTS, half_w):
                prata += 1
            else:
                d, _ = bolt_distance(vx, vy)
                if d <= BOLT_GLOW_R:
                    brilho += (1.0 - d / BOLT_GLOW_R) ** 2

    if brilho > 0:
        a = min(0.55, brilho / 9.0 * 1.8)
        r, g, b = mix((r, g, b), BOLT_GLOW, a)

    if prata > 0:
        # Gradiente do prata pela altura da marca, não do ícone.
        _, vy = to_view(size, x + 0.5, y + 0.5)
        k = min(1.0, max(0.0, (vy - 22.0) / 58.0))
        cor = mix(MARK_TOP, MARK_BOTTOM, k)
        a = min(1.0, prata / 9.0)
        r, g, b = mix((r, g, b), cor, a)

    if raio > 0:
        a = min(1.0, raio / 9.0)
        r, g, b = mix((r, g, b), BOLT_CORE, a)

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


# ------------------------------------------------------------------- SVG ---


def ring_path() -> str:
    x1, y1 = ring_point(RING_FROM)
    x2, y2 = ring_point(RING_TO)
    grande = 1 if (RING_TO - RING_FROM) > 180 else 0
    return f"M{x1:.1f} {y1:.1f} A{RING_R:.1f} {RING_R:.1f} 0 {grande} 1 {x2:.1f} {y2:.1f}"


def w_path() -> str:
    return " ".join(
        f"{'M' if i == 0 else 'L'}{x:.1f} {y:.1f}" for i, (x, y) in enumerate(W_POINTS)
    )


def bolt_path() -> str:
    (x1, y1), (x2, y2) = BOLT_A, BOLT_B
    dx, dy = x2 - x1, y2 - y1
    n = math.hypot(dx, dy)
    nx, ny = -dy / n * BOLT_HALF, dx / n * BOLT_HALF
    mx, my = (x1 + x2) / 2, (y1 + y2) / 2
    return (
        f"M{x1:.1f} {y1:.1f} L{mx + nx:.1f} {my + ny:.1f} "
        f"L{x2:.1f} {y2:.1f} L{mx - nx:.1f} {my - ny:.1f} Z"
    )


def mark_svg(background: bool) -> str:
    fundo = (
        '  <rect width="100" height="100" rx="22" fill="url(#bg)"/>\n'
        if background
        else ""
    )
    defs_bg = (
        '    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">\n'
        f'      <stop offset="0" stop-color="#{BG_TOP[0]:02x}{BG_TOP[1]:02x}{BG_TOP[2]:02x}"/>\n'
        f'      <stop offset="1" stop-color="#{BG_BOTTOM[0]:02x}{BG_BOTTOM[1]:02x}{BG_BOTTOM[2]:02x}"/>\n'
        "    </linearGradient>\n"
        if background
        else ""
    )
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <defs>
{defs_bg}    <linearGradient id="prata" x1="0.2" y1="0" x2="0.8" y2="1">
      <stop offset="0" stop-color="#fcfdff"/>
      <stop offset="1" stop-color="#929aac"/>
    </linearGradient>
    <filter id="brilho" x="-30%" y="-30%" width="160%" height="160%">
      <feGaussianBlur stdDeviation="2.4"/>
    </filter>
  </defs>
{fundo}  <path d="{bolt_path()}" fill="#2b2bff" filter="url(#brilho)" opacity="0.85"/>
  <g fill="none" stroke="url(#prata)" stroke-width="{RING_W:.1f}" stroke-linecap="butt" stroke-linejoin="round">
    <path d="{ring_path()}"/>
    <path d="{w_path()}"/>
  </g>
  <path d="{bolt_path()}" fill="#8f8fff"/>
</svg>
"""


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    PUBLIC.mkdir(parents=True, exist_ok=True)
    BRAND.mkdir(parents=True, exist_ok=True)

    sizes = {32: "32x32.png", 128: "128x128.png", 256: "128x128@2x.png", 512: "icon.png"}
    pngs = {}
    for size, name in sizes.items():
        data = make_png(size)
        (OUT / name).write_bytes(data)
        pngs[size] = data
    (OUT / "icon.ico").write_bytes(make_ico({s: pngs[s] for s in (32, 128, 256)}))

    (PUBLIC / "favicon.svg").write_text(mark_svg(background=False), encoding="utf-8")
    (BRAND / "mark.svg").write_text(mark_svg(background=False), encoding="utf-8")
    (BRAND / "icon.svg").write_text(mark_svg(background=True), encoding="utf-8")
    for size in (16, 24, 32, 48, 64, 128, 256, 1024):
        (BRAND / f"logo-{size}.png").write_bytes(
            pngs.get(size) or make_png(size)
        )
    print(f"ícones do OpenWeights gerados em {OUT} e {BRAND}")


if __name__ == "__main__":
    main()
