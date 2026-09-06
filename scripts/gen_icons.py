#!/usr/bin/env python3
"""Ícones e arquivos de marca do OpenWeights.

A marca é um monograma O+W. Um anel prata ABERTO — o "open", o modelo que dá
para enxergar por dentro — desce pela esquerda e, sem emenda, vira uma fita
que serpenteia como um W: roxo embaixo, ciano quando sobe. Do braço direito
soltam-se quatro cubos, os pesos saindo do modelo.

Geometria medida na folha de identidade (art/referencia.png).

Este arquivo é a ÚNICA fonte da forma. Ele gera:
  - src-tauri/icons/*.png e icon.ico  (ícones do app e da barra de tarefas)
  - public/favicon.svg                (aba do navegador)
  - brand/*.svg e brand/logo-*.png    (arquivos de marca)
  - src/components/brandArt.ts        (o que o React desenha)

Sem dependências externas: o rasterizador é um scanline com winding nonzero,
cobertura analítica na horizontal e 4 subamostras na vertical.
"""

from __future__ import annotations

import math
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "src-tauri" / "icons"
PUBLIC = ROOT / "public"
BRAND = ROOT / "brand"
GEN_TS = ROOT / "src" / "components" / "brandArt.ts"

# ----------------------------------------------------------------- cores ---
# Amostradas na folha de identidade. A fita é o gradiente da marca inteira:
# tudo que o app pinta de accent sai daqui.

ROXO = (0x7B, 0x5C, 0xFF)
AZUL = (0x3D, 0x8B, 0xFF)
CIANO = (0x00, 0xD8, 0xFC)

#: Fita do W, do roxo ao ciano ao longo da diagonal.
FITA_STOPS = [
    (0.0, ROXO),
    (0.34, (0x5A, 0x6B, 0xFF)),
    (0.60, AZUL),
    (0.84, CIANO),
    (1.0, (0x3D, 0xEB, 0xFF)),
]
#: Prata do anel: branco no alto, azul-acinzentado embaixo.
PRATA_STOPS = [
    (0.0, (0xFF, 0xFF, 0xFF)),
    (0.46, (0xE9, 0xEF, 0xFB)),
    (1.0, (0x93, 0xA4, 0xCB)),
]

#: Fundo do ícone quadrado (o favicon e a marca solta saem sem fundo).
BG_TOP = (0x14, 0x17, 0x24)
BG_BOTTOM = (0x08, 0x0A, 0x12)
#: Os dois brilhos que a folha põe atrás da marca.
GLOW = [((30.0, 84.0), 46.0, ROXO, 0.30), ((76.0, 34.0), 40.0, CIANO, 0.22)]

#: Quanto a lateral da fita e do anel escurece — é o que dá volume sem sombra.
SOMBRA_FITA = 0.50
SOMBRA_PRATA = 0.46
#: Deslocamento da lateral, em unidades do viewBox.
RELEVO = 1.4

# ------------------------------------------------------------- geometria ---
# Tudo em coordenadas de trabalho; no fim `Geo.fit()` reenquadra no viewBox.

VIEW = 100.0


def _pt(cx: float, cy: float, r: float, deg: float) -> tuple[float, float]:
    a = math.radians(deg)
    return (cx + r * math.cos(a), cy + r * math.sin(a))


class Geo:
    """A marca em números. Uma instância é um enquadramento."""

    def __init__(self) -> None:
        # Anel: círculo de raio médio R e espessura BAND, vivo só entre os dois
        # ângulos. O que falta — o quadrante de baixo e a direita — é a
        # abertura por onde o W passa e sobe.
        self.cx, self.cy = 51.5, 56.0
        self.r = 34.2
        self.band = 15.6
        self.arc_from, self.arc_to = 147.0, 300.0
        #: Comprimento do bico que corta a ponta de cima do anel em diagonal.
        self.bico = 12.0

        # A fita começa exatamente na ponta de baixo do anel, com a mesma
        # tangente: de longe é um traço só.
        vale_esq = (27.5, 92.0)
        pico = (51.5, 46.0)
        vale_dir = (63.5, 86.0)
        ponta = (87.0, 43.0)
        self.fita = [self._ring_pt(self.arc_from), vale_esq, pico, vale_dir, ponta]
        self.fita_t0 = (math.sin(math.radians(self.arc_from)), -math.cos(math.radians(self.arc_from)))

        #: Eixo do gradiente da fita: da base roxa ao braço ciano.
        self.grad_a = (20.0, 95.0)
        self.grad_b = (88.0, 40.0)

        # Cubos: (centro, meia-largura, altura da lateral, cor).
        self.cubos = [
            ((80.5, 14.5), 5.6, 5.0, CIANO),
            ((67.5, 26.5), 6.6, 6.0, ROXO),
            ((88.0, 27.5), 6.6, 6.0, (0x4C, 0x6B, 0xFF)),
            ((70.5, 40.5), 7.8, 7.0, (0x18, 0xAE, 0xFF)),
        ]

    # -- transformação ----------------------------------------------------

    def _ring_pt(self, deg: float) -> tuple[float, float]:
        return _pt(self.cx, self.cy, self.r, deg)

    def scaled(self, s: float, tx: float, ty: float) -> "Geo":
        g = Geo()
        m = lambda p: (p[0] * s + tx, p[1] * s + ty)  # noqa: E731
        g.cx, g.cy = m((self.cx, self.cy))
        g.r = self.r * s
        g.band = self.band * s
        g.arc_from, g.arc_to = self.arc_from, self.arc_to
        g.bico = self.bico * s
        g.fita = [m(p) for p in self.fita]
        g.fita_t0 = self.fita_t0
        g.grad_a, g.grad_b = m(self.grad_a), m(self.grad_b)
        g.cubos = [(m(c), w * s, h * s, cor) for c, w, h, cor in self.cubos]
        return g

    def fit(self, box: float, margem: float) -> "Geo":
        """Reenquadra a marca inteira numa caixa quadrada, com margem."""
        polys = [p for peca in self.pecas() for p in peca.polys]
        xs = [x for poly in polys for x, _ in poly]
        ys = [y for poly in polys for _, y in poly]
        w, h = max(xs) - min(xs), max(ys) - min(ys)
        alvo = box - 2 * margem
        s = alvo / max(w, h)
        return self.scaled(
            s,
            margem + (alvo - w * s) / 2 - min(xs) * s,
            margem + (alvo - h * s) / 2 - min(ys) * s,
        )

    # -- as peças, na ordem de pintura ------------------------------------

    def curva_fita(self) -> list[tuple[float, float]]:
        return flatten(catmull(self.fita, self.fita_t0))

    def arco_pts(self) -> list[tuple[float, float]]:
        n = 96
        return [
            self._ring_pt(self.arc_to + (self.arc_from - self.arc_to) * i / n)
            for i in range(n + 1)
        ]

    def bico_poly(self) -> list[tuple[float, float]]:
        """A ponta de cima do anel, cortada em diagonal."""
        half = self.band / 2
        u = _pt(0.0, 0.0, 1.0, self.arc_to)
        t = (-math.sin(math.radians(self.arc_to)), math.cos(math.radians(self.arc_to)))
        fora = (self.cx + u[0] * (self.r + half), self.cy + u[1] * (self.r + half))
        dentro = (self.cx + u[0] * (self.r - half), self.cy + u[1] * (self.r - half))
        ponta = (fora[0] + t[0] * self.bico, fora[1] + t[1] * self.bico)
        return orient([fora, ponta, dentro])

    def pecas(self) -> list["Peca"]:
        prata = Paint(
            "prata",
            (self.cx - self.r * 0.55, self.cy - self.r * 1.1),
            (self.cx - self.r * 0.15, self.cy + self.r * 1.15),
            PRATA_STOPS,
        )
        fita = Paint("fita", self.grad_a, self.grad_b, FITA_STOPS)
        prata_baixo = prata.escurecido("prataBaixo", SOMBRA_PRATA)
        fita_baixo = fita.escurecido("fitaBaixo", SOMBRA_FITA)

        arco, curva, bico = self.arco_pts(), self.curva_fita(), self.bico_poly()
        d = RELEVO * (self.band / Geo().band)
        desce = lambda pts: [(x, y + d) for x, y in pts]  # noqa: E731

        pecas = [
            Peca.stroke(desce(arco), self.band, prata_baixo, svg=self.arco_d(dy=d)),
            Peca.fill([desce(bico)], prata_baixo, svg=poly_d(desce(bico))),
            Peca.stroke(desce(curva), self.band, fita_baixo, svg=self.fita_d(dy=d)),
            Peca.stroke(arco, self.band, prata, svg=self.arco_d()),
            Peca.fill([bico], prata, svg=poly_d(bico)),
            Peca.stroke(curva, self.band, fita, svg=self.fita_d()),
        ]
        for i, (c, w, h, cor) in enumerate(self.cubos):
            for face, tom, poly in cube_faces(c, w, h):
                pecas.append(
                    Peca.fill(
                        [poly],
                        Paint.solido(f"cubo{i}{face}", escurece(cor, tom)),
                        svg=poly_d(poly),
                    )
                )
        return pecas

    # -- os mesmos traços, em `d` de SVG ----------------------------------

    def arco_d(self, dy: float = 0.0) -> str:
        x1, y1 = self._ring_pt(self.arc_to)
        x2, y2 = self._ring_pt(self.arc_from)
        grande = 1 if abs(self.arc_to - self.arc_from) > 180 else 0
        return (
            f"M{x1:.2f} {y1 + dy:.2f} "
            f"A{self.r:.2f} {self.r:.2f} 0 {grande} 0 {x2:.2f} {y2 + dy:.2f}"
        )

    def fita_d(self, dy: float = 0.0) -> str:
        segs = catmull(self.fita, self.fita_t0)
        (p0, _, _, _) = segs[0]
        out = [f"M{p0[0]:.2f} {p0[1] + dy:.2f}"]
        for _, c1, c2, p3 in segs:
            out.append(
                f"C{c1[0]:.2f} {c1[1] + dy:.2f} {c2[0]:.2f} {c2[1] + dy:.2f} "
                f"{p3[0]:.2f} {p3[1] + dy:.2f}"
            )
        return " ".join(out)


# ------------------------------------------------------------- curvas ------


def catmull(pts, t0=None, tension: float = 0.75):
    """Catmull-Rom pelos pontos, devolvida em cúbicas de Bézier."""
    n = len(pts)
    tang = []
    for i in range(n):
        if i == 0:
            if t0 is not None:
                dx, dy = pts[1][0] - pts[0][0], pts[1][1] - pts[0][1]
                comp = math.hypot(dx, dy)
                tang.append((t0[0] * comp, t0[1] * comp))
            else:
                tang.append((pts[1][0] - pts[0][0], pts[1][1] - pts[0][1]))
        elif i == n - 1:
            tang.append((pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]))
        else:
            tang.append(
                (
                    (pts[i + 1][0] - pts[i - 1][0]) * tension / 2,
                    (pts[i + 1][1] - pts[i - 1][1]) * tension / 2,
                )
            )
    segs = []
    for i in range(n - 1):
        p0, p1 = pts[i], pts[i + 1]
        m0, m1 = tang[i], tang[i + 1]
        segs.append(
            (
                p0,
                (p0[0] + m0[0] / 3, p0[1] + m0[1] / 3),
                (p1[0] - m1[0] / 3, p1[1] - m1[1] / 3),
                p1,
            )
        )
    return segs


def flatten(segs, por_seg: int = 40) -> list[tuple[float, float]]:
    pts = [segs[0][0]]
    for p0, c1, c2, p3 in segs:
        for i in range(1, por_seg + 1):
            t = i / por_seg
            u = 1 - t
            pts.append(
                (
                    u * u * u * p0[0] + 3 * u * u * t * c1[0] + 3 * u * t * t * c2[0] + t * t * t * p3[0],
                    u * u * u * p0[1] + 3 * u * u * t * c1[1] + 3 * u * t * t * c2[1] + t * t * t * p3[1],
                )
            )
    return pts


# ------------------------------------------------------------- polígonos ---


def area(poly) -> float:
    s = 0.0
    for i in range(len(poly)):
        x0, y0 = poly[i]
        x1, y1 = poly[(i + 1) % len(poly)]
        s += x0 * y1 - x1 * y0
    return s / 2


def orient(poly):
    """Todos os polígonos no mesmo sentido: é assim que o winding nonzero
    vira união em vez de furo."""
    return poly if area(poly) > 0 else poly[::-1]


def stroke_polys(pts, largura: float) -> list[list[tuple[float, float]]]:
    """Traço grosso virado em polígonos: um quadrilátero por segmento e um
    disco em cada junta. Com todos no mesmo sentido, o nonzero soma tudo
    num contorno só — sem precisar calcular offset de curva."""
    half = largura / 2
    polys = []
    for i in range(len(pts) - 1):
        (x0, y0), (x1, y1) = pts[i], pts[i + 1]
        dx, dy = x1 - x0, y1 - y0
        n = math.hypot(dx, dy)
        if n < 1e-9:
            continue
        nx, ny = -dy / n * half, dx / n * half
        polys.append(
            orient([(x0 + nx, y0 + ny), (x1 + nx, y1 + ny), (x1 - nx, y1 - ny), (x0 - nx, y0 - ny)])
        )
    lados = 10
    for x, y in pts[1:-1]:
        polys.append(
            orient(
                [
                    (x + half * math.cos(2 * math.pi * k / lados), y + half * math.sin(2 * math.pi * k / lados))
                    for k in range(lados)
                ]
            )
        )
    return polys


def cube_faces(centro, w: float, h: float):
    """Um cubo isométrico em três faces: topo, esquerda, direita."""
    cx, cy = centro
    m = w * 0.5774  # 30°, a inclinação do isométrico
    topo = (cx, cy - h / 2 - m)
    dir_ = (cx + w, cy - h / 2)
    esq = (cx - w, cy - h / 2)
    meio = (cx, cy - h / 2 + m)
    bdir = (cx + w, cy + h / 2)
    besq = (cx - w, cy + h / 2)
    baixo = (cx, cy + h / 2 + m)
    return [
        ("Topo", 1.0, orient([topo, dir_, meio, esq])),
        ("Esq", 0.62, orient([esq, meio, baixo, besq])),
        ("Dir", 0.40, orient([meio, dir_, bdir, baixo])),
    ]


def poly_d(poly) -> str:
    return "M" + " L".join(f"{x:.2f} {y:.2f}" for x, y in poly) + " Z"


# ----------------------------------------------------------------- tinta ---


def escurece(cor, k: float):
    return tuple(int(c * k) for c in cor)


def hexa(cor) -> str:
    return "#%02x%02x%02x" % cor


class Paint:
    """Gradiente linear (ou cor sólida) que sabe se pintar nos dois destinos:
    por pixel no rasterizador, por `<linearGradient>` no SVG."""

    def __init__(self, nome, p0, p1, stops):
        self.nome, self.p0, self.p1, self.stops = nome, p0, p1, stops

    @staticmethod
    def solido(nome, cor):
        return Paint(nome, (0, 0), (1, 0), [(0.0, cor), (1.0, cor)])

    @property
    def solida(self) -> bool:
        return self.stops[0][1] == self.stops[-1][1]

    def escurecido(self, nome, k: float):
        return Paint(nome, self.p0, self.p1, [(t, escurece(c, k)) for t, c in self.stops])

    def cor_em(self, x: float, y: float):
        if self.solida:
            return self.stops[0][1]
        ax, ay = self.p0
        bx, by = self.p1
        dx, dy = bx - ax, by - ay
        t = ((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)
        t = min(1.0, max(0.0, t))
        for i in range(len(self.stops) - 1):
            t0, c0 = self.stops[i]
            t1, c1 = self.stops[i + 1]
            if t <= t1 or i == len(self.stops) - 2:
                k = 0.0 if t1 == t0 else (t - t0) / (t1 - t0)
                k = min(1.0, max(0.0, k))
                return tuple(int(c0[j] + (c1[j] - c0[j]) * k) for j in range(3))
        return self.stops[-1][1]

    def svg_def(self) -> str:
        if self.solida:
            return ""
        paradas = "".join(
            f'\n      <stop offset="{t:g}" stop-color="{hexa(c)}"/>' for t, c in self.stops
        )
        return (
            f'    <linearGradient id="{self.nome}" gradientUnits="userSpaceOnUse" '
            f'x1="{self.p0[0]:.2f}" y1="{self.p0[1]:.2f}" '
            f'x2="{self.p1[0]:.2f}" y2="{self.p1[1]:.2f}">{paradas}\n'
            "    </linearGradient>\n"
        )

    def svg_ref(self) -> str:
        return hexa(self.stops[0][1]) if self.solida else f"url(#{self.nome})"


class Peca:
    """Uma forma: os polígonos que o rasterizador usa e o `d` que o SVG usa."""

    def __init__(self, polys, paint, svg, largura=None):
        self.polys, self.paint, self.svg, self.largura = polys, paint, svg, largura

    @staticmethod
    def fill(polys, paint, svg):
        return Peca([orient(p) for p in polys], paint, svg)

    @staticmethod
    def stroke(pts, largura, paint, svg):
        return Peca(stroke_polys(pts, largura), paint, svg, largura)


# ------------------------------------------------------------ rasteriza ----

SS = 4  # subamostras verticais


def cobertura(polys, size: int) -> tuple[dict, int, int]:
    """Cobertura por pixel (0..1) de um conjunto de polígonos, nonzero."""
    arestas = []
    for poly in polys:
        n = len(poly)
        for i in range(n):
            x0, y0 = poly[i]
            x1, y1 = poly[(i + 1) % n]
            if y0 == y1:
                continue
            arestas.append((min(y0, y1), max(y0, y1), x0, y0, (x1 - x0) / (y1 - y0), 1 if y1 > y0 else -1))
    if not arestas:
        return {}, 0, -1

    linhas = [[] for _ in range(size)]
    for a in arestas:
        lo = max(0, int(math.floor(a[0])))
        hi = min(size - 1, int(math.ceil(a[1])))
        for py in range(lo, hi + 1):
            linhas[py].append(a)

    cov: dict[int, float] = {}
    peso = 1.0 / SS
    y0lim = next((i for i, l in enumerate(linhas) if l), 0)
    y1lim = max((i for i, l in enumerate(linhas) if l), default=-1)
    for py in range(y0lim, y1lim + 1):
        ativas = linhas[py]
        if not ativas:
            continue
        base = py * size
        for k in range(SS):
            y = py + (k + 0.5) * peso
            xs = []
            for ymin, ymax, x0, ey0, inc, w in ativas:
                if ymin <= y < ymax:
                    xs.append((x0 + (y - ey0) * inc, w))
            if not xs:
                continue
            xs.sort()
            wind = 0
            inicio = 0.0
            for x, w in xs:
                antes = wind
                wind += w
                if antes == 0 and wind != 0:
                    inicio = x
                elif antes != 0 and wind == 0:
                    _span(cov, base, size, inicio, x, peso)
    return cov, y0lim, y1lim


def _span(cov: dict, base: int, size: int, xa: float, xb: float, peso: float) -> None:
    xa = max(xa, 0.0)
    xb = min(xb, float(size))
    if xb <= xa:
        return
    i0 = int(xa)
    i1 = min(int(math.ceil(xb)) - 1, size - 1)
    if i0 == i1:
        cov[base + i0] = cov.get(base + i0, 0.0) + (xb - xa) * peso
        return
    cov[base + i0] = cov.get(base + i0, 0.0) + (i0 + 1 - xa) * peso
    for i in range(i0 + 1, i1):
        cov[base + i] = cov.get(base + i, 0.0) + peso
    cov[base + i1] = cov.get(base + i1, 0.0) + (xb - i1) * peso


def rounded(size: int, x: int, y: int) -> float:
    """Cobertura do quadrado de cantos redondos do ícone (raio 22%)."""
    r = size * 0.22
    lo, hi = r, size - r
    acc = 0.0
    for sx in (0.25, 0.75):
        for sy in (0.25, 0.75):
            px, py = x + sx, y + sy
            cx = min(max(px, lo), hi)
            cy = min(max(py, lo), hi)
            if (px - cx) ** 2 + (py - cy) ** 2 <= r * r:
                acc += 0.25
    return acc


def fundo(size: int, x: int, y: int, esc: float) -> tuple[float, float, float]:
    t = y / max(size - 1, 1)
    r = BG_TOP[0] + (BG_BOTTOM[0] - BG_TOP[0]) * t
    g = BG_TOP[1] + (BG_BOTTOM[1] - BG_TOP[1]) * t
    b = BG_TOP[2] + (BG_BOTTOM[2] - BG_TOP[2]) * t
    vx, vy = (x + 0.5) / esc, (y + 0.5) / esc
    for (gx, gy), raio, cor, forca in GLOW:
        d = math.hypot(vx - gx, vy - gy)
        if d < raio:
            a = (1 - d / raio) ** 2 * forca
            r += (cor[0] - r) * a
            g += (cor[1] - g) * a
            b += (cor[2] - b) * a
    return (r, g, b)


def desenha(size: int, com_fundo: bool) -> bytes:
    margem = 0.13 if com_fundo else 0.02
    geo = Geo().fit(float(size), size * margem)
    esc = size / VIEW

    px = [0.0] * (size * size * 4)  # r,g,b,a pré-multiplicado não; direto
    if com_fundo:
        for y in range(size):
            for x in range(size):
                a = rounded(size, x, y)
                if a <= 0:
                    continue
                r, g, b = fundo(size, x, y, esc)
                i = (y * size + x) * 4
                px[i], px[i + 1], px[i + 2], px[i + 3] = r, g, b, a

    for peca in geo.pecas():
        cov, _, _ = cobertura(peca.polys, size)
        for idx, a in cov.items():
            if a <= 0.002:
                continue
            a = min(1.0, a)
            x, y = idx % size, idx // size
            if com_fundo:
                a *= rounded(size, x, y)
                if a <= 0:
                    continue
            # A geometria já veio em pixels (`fit` recebeu o tamanho do
            # ícone), então o gradiente também se lê em pixels.
            cor = peca.paint.cor_em(x + 0.5, y + 0.5)
            i = idx * 4
            na = px[i + 3] + a * (1 - px[i + 3])
            if na <= 0:
                continue
            for c in range(3):
                px[i + c] = (px[i + c] * px[i + 3] * (1 - a) + cor[c] * a) / na
            px[i + 3] = na

    raw = bytearray()
    for y in range(size):
        raw.append(0)
        for x in range(size):
            i = (y * size + x) * 4
            raw.extend(
                (
                    max(0, min(255, int(px[i] + 0.5))),
                    max(0, min(255, int(px[i + 1] + 0.5))),
                    max(0, min(255, int(px[i + 2] + 0.5))),
                    max(0, min(255, int(px[i + 3] * 255 + 0.5))),
                )
            )
    return png(raw, size)


def png(raw: bytes, size: int) -> bytes:
    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def ico(pngs: dict[int, bytes]) -> bytes:
    entradas = sorted(pngs.items())
    cab = struct.pack("<HHH", 0, 1, len(entradas))
    dirs = b""
    blobs = b""
    off = 6 + 16 * len(entradas)
    for size, data in entradas:
        s = 0 if size >= 256 else size
        dirs += struct.pack("<BBBBHHII", s, s, 0, 0, 1, 32, len(data), off)
        blobs += data
        off += len(data)
    return cab + dirs + blobs


# ------------------------------------------------------------------- SVG ---


def svg(com_fundo: bool, tema: bool = False) -> str:
    """`tema=True` troca o prata por currentColor: é a versão que o app usa,
    para o anel acompanhar o texto no claro e no escuro."""
    geo = Geo().fit(VIEW, VIEW * (0.13 if com_fundo else 0.02))
    pecas = geo.pecas()

    defs = []
    vistos = set()
    if com_fundo:
        defs.append(
            '    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">\n'
            f'      <stop offset="0" stop-color="{hexa(BG_TOP)}"/>\n'
            f'      <stop offset="1" stop-color="{hexa(BG_BOTTOM)}"/>\n'
            "    </linearGradient>\n"
        )
    for p in pecas:
        d = p.paint.svg_def()
        if d and p.paint.nome not in vistos:
            vistos.add(p.paint.nome)
            defs.append(d)

    corpo = []
    if com_fundo:
        corpo.append('  <rect width="100" height="100" rx="22" fill="url(#bg)"/>')
    for p in pecas:
        if p.largura:
            corpo.append(
                f'  <path d="{p.svg}" fill="none" stroke="{p.paint.svg_ref()}" '
                f'stroke-width="{p.largura:.2f}" stroke-linecap="butt" stroke-linejoin="round"/>'
            )
        else:
            corpo.append(f'  <path d="{p.svg}" fill="{p.paint.svg_ref()}"/>')

    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">\n'
        "  <defs>\n" + "".join(defs) + "  </defs>\n" + "\n".join(corpo) + "\n</svg>\n"
    )


def wordmark_svg() -> str:
    """Marca + nome, para README e loja. O nome sai em texto do sistema para
    o arquivo não depender de fonte embutida."""
    geo = Geo().fit(100.0, 2.0)
    pecas = geo.pecas()
    defs = []
    vistos = set()
    for p in pecas:
        d = p.paint.svg_def()
        if d and p.paint.nome not in vistos:
            vistos.add(p.paint.nome)
            defs.append(d)
    defs.append(
        '    <linearGradient id="nome" x1="0" y1="0" x2="1" y2="0">\n'
        f'      <stop offset="0" stop-color="{hexa(CIANO)}"/>\n'
        f'      <stop offset="1" stop-color="{hexa(ROXO)}"/>\n'
        "    </linearGradient>\n"
    )
    corpo = []
    for p in pecas:
        if p.largura:
            corpo.append(
                f'    <path d="{p.svg}" fill="none" stroke="{p.paint.svg_ref()}" '
                f'stroke-width="{p.largura:.2f}" stroke-linecap="butt" stroke-linejoin="round"/>'
            )
        else:
            corpo.append(f'    <path d="{p.svg}" fill="{p.paint.svg_ref()}"/>')
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 420 100">\n'
        "  <defs>\n" + "".join(defs) + "  </defs>\n"
        "  <g>\n" + "\n".join(corpo) + "\n  </g>\n"
        '  <text x="118" y="68" font-family="Segoe UI, system-ui, sans-serif" '
        'font-size="54" font-weight="600" letter-spacing="-1">'
        '<tspan fill="#ffffff">Open</tspan>'
        '<tspan fill="url(#nome)">Weights</tspan></text>\n'
        "</svg>\n"
    )


# -------------------------------------------------------------- React ------


def brand_ts() -> str:
    """O mesmo desenho, em dados para o componente React."""
    geo = Geo().fit(VIEW, 2.0)
    pecas = geo.pecas()

    def linha(p: Peca) -> str:
        tipo = "stroke" if p.largura else "fill"
        largura = f", w: {p.largura:.2f}" if p.largura else ""
        return f'  {{ kind: "{tipo}", d: "{p.svg}", paint: "{p.paint.nome}"{largura} }},'

    tintas = {}
    for p in pecas:
        tintas.setdefault(p.paint.nome, p.paint)

    def tinta(nome: str, paint: Paint) -> str:
        if paint.solida:
            return f'  {nome}: {{ solid: "{hexa(paint.stops[0][1])}" }},'
        paradas = ", ".join(f'{{ at: {t:g}, color: "{hexa(c)}" }}' for t, c in paint.stops)
        return (
            f"  {nome}: {{ x1: {paint.p0[0]:.2f}, y1: {paint.p0[1]:.2f}, "
            f"x2: {paint.p1[0]:.2f}, y2: {paint.p1[1]:.2f}, stops: [{paradas}] }},"
        )

    return (
        "// Gerado por scripts/gen_icons.py — não edite à mão.\n"
        "//\n"
        "// A geometria da marca vive lá: anel prata aberto, a fita do W e os\n"
        "// cubos. Rode o script depois de mexer na forma.\n\n"
        "export type BrandStop = { at: number; color: string };\n"
        "export type BrandPaint =\n"
        "  | { solid: string }\n"
        "  | { x1: number; y1: number; x2: number; y2: number; stops: BrandStop[] };\n"
        "export type BrandPiece = {\n"
        '  kind: "fill" | "stroke";\n'
        "  d: string;\n"
        "  paint: string;\n"
        "  w?: number;\n"
        "};\n\n"
        "export const BRAND_PAINTS: Record<string, BrandPaint> = {\n"
        + "\n".join(tinta(n, p) for n, p in tintas.items())
        + "\n};\n\n"
        "export const BRAND_PIECES: BrandPiece[] = [\n"
        + "\n".join(linha(p) for p in pecas)
        + "\n];\n\n"
        "/** As tintas do anel — o app troca por currentColor para seguir o tema. */\n"
        'export const RING_PAINTS = ["prata", "prataBaixo"] as const;\n'
    )


# ------------------------------------------------------------------ main ---


def main() -> None:
    for d in (ICONS, PUBLIC, BRAND):
        d.mkdir(parents=True, exist_ok=True)

    tamanhos = {32: "32x32.png", 128: "128x128.png", 256: "128x128@2x.png", 512: "icon.png"}
    pngs = {}
    for size, nome in tamanhos.items():
        data = desenha(size, com_fundo=True)
        (ICONS / nome).write_bytes(data)
        pngs[size] = data
        print(f"  {nome}")
    (ICONS / "icon.ico").write_bytes(ico({s: pngs[s] for s in (32, 128, 256)}))

    (PUBLIC / "favicon.svg").write_text(svg(com_fundo=False), encoding="utf-8")
    (BRAND / "mark.svg").write_text(svg(com_fundo=False), encoding="utf-8")
    (BRAND / "icon.svg").write_text(svg(com_fundo=True), encoding="utf-8")
    (BRAND / "wordmark.svg").write_text(wordmark_svg(), encoding="utf-8")
    GEN_TS.write_text(brand_ts(), encoding="utf-8")

    for size in (16, 24, 32, 48, 64, 128, 256, 1024):
        (BRAND / f"logo-{size}.png").write_bytes(pngs.get(size) or desenha(size, com_fundo=True))
        print(f"  brand/logo-{size}.png")
    print(f"marca do OpenWeights gerada em {ICONS}, {PUBLIC} e {BRAND}")


if __name__ == "__main__":
    main()
