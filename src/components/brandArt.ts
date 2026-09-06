// Gerado por scripts/gen_icons.py — não edite à mão.
//
// A geometria da marca vive lá: anel prata aberto, a fita do W e os
// cubos. Rode o script depois de mexer na forma.

export type BrandStop = { at: number; color: string };
export type BrandPaint =
  | { solid: string }
  | { x1: number; y1: number; x2: number; y2: number; stops: BrandStop[] };
export type BrandPiece = {
  kind: "fill" | "stroke";
  d: string;
  paint: string;
  w?: number;
};

export const BRAND_PAINTS: Record<string, BrandPaint> = {
  prataBaixo: { x1: 29.99, y1: 11.94, x2: 44.13, y2: 91.48, stops: [{ at: 0, color: "#757575" }, { at: 0.46, color: "#6b6d73" }, { at: 1, color: "#434b5d" }] },
  fitaBaixo: { x1: 16.87, y1: 91.14, x2: 87.16, y2: 34.29, stops: [{ at: 0, color: "#3d2e7f" }, { at: 0.34, color: "#2d357f" }, { at: 0.6, color: "#1e457f" }, { at: 0.84, color: "#006c7e" }, { at: 1, color: "#1e757f" }] },
  prata: { x1: 29.99, y1: 11.94, x2: 44.13, y2: 91.48, stops: [{ at: 0, color: "#ffffff" }, { at: 0.46, color: "#e9effb" }, { at: 1, color: "#93a4cb" }] },
  fita: { x1: 16.87, y1: 91.14, x2: 87.16, y2: 34.29, stops: [{ at: 0, color: "#7b5cff" }, { at: 0.34, color: "#5a6bff" }, { at: 0.6, color: "#3d8bff" }, { at: 0.84, color: "#00d8fc" }, { at: 1, color: "#3debff" }] },
  cubo0Topo: { solid: "#00d8fc" },
  cubo0Esq: { solid: "#00859c" },
  cubo0Dir: { solid: "#005664" },
  cubo1Topo: { solid: "#7b5cff" },
  cubo1Esq: { solid: "#4c399e" },
  cubo1Dir: { solid: "#312466" },
  cubo2Topo: { solid: "#4c6bff" },
  cubo2Esq: { solid: "#2f429e" },
  cubo2Dir: { solid: "#1e2a66" },
  cubo3Topo: { solid: "#18aeff" },
  cubo3Esq: { solid: "#0e6b9e" },
  cubo3Dir: { solid: "#094566" },
};

export const BRAND_PIECES: BrandPiece[] = [
  { kind: "stroke", d: "M67.11 21.66 A35.35 35.35 0 0 0 19.78 71.53", paint: "prataBaixo", w: 16.13 },
  { kind: "fill", d: "M71.14 14.67 L81.88 20.88 L63.08 28.64 Z", paint: "prataBaixo" },
  { kind: "stroke", d: "M19.78 71.53 C23.16 76.73 20.91 93.19 24.62 89.49 C28.33 85.79 44.78 42.71 49.43 41.94 C54.08 41.16 57.25 83.67 61.84 83.29 C66.42 82.90 78.03 53.65 86.13 38.84", paint: "fitaBaixo", w: 16.13 },
  { kind: "stroke", d: "M67.11 20.21 A35.35 35.35 0 0 0 19.78 70.08", paint: "prata", w: 16.13 },
  { kind: "fill", d: "M71.14 13.23 L81.88 19.43 L63.08 27.19 Z", paint: "prata" },
  { kind: "stroke", d: "M19.78 70.08 C23.16 75.28 20.91 91.74 24.62 88.04 C28.33 84.34 44.78 41.27 49.43 40.49 C54.08 39.71 57.25 82.23 61.84 81.84 C66.42 81.45 78.03 52.21 86.13 37.39", paint: "fita", w: 16.13 },
  { kind: "fill", d: "M79.41 2.00 L85.20 5.34 L79.41 8.69 L73.62 5.34 Z", paint: "cubo0Topo" },
  { kind: "fill", d: "M73.62 5.34 L79.41 8.69 L79.41 13.85 L73.62 10.51 Z", paint: "cubo0Esq" },
  { kind: "fill", d: "M79.41 8.69 L85.20 5.34 L85.20 10.51 L79.41 13.85 Z", paint: "cubo0Dir" },
  { kind: "fill", d: "M65.97 13.29 L72.79 17.23 L65.97 21.17 L59.15 17.23 Z", paint: "cubo1Topo" },
  { kind: "fill", d: "M59.15 17.23 L65.97 21.17 L65.97 27.37 L59.15 23.43 Z", paint: "cubo1Esq" },
  { kind: "fill", d: "M65.97 21.17 L72.79 17.23 L72.79 23.43 L65.97 27.37 Z", paint: "cubo1Dir" },
  { kind: "fill", d: "M87.16 14.32 L93.99 18.26 L87.16 22.20 L80.34 18.26 Z", paint: "cubo2Topo" },
  { kind: "fill", d: "M80.34 18.26 L87.16 22.20 L87.16 28.41 L80.34 24.47 Z", paint: "cubo2Esq" },
  { kind: "fill", d: "M87.16 22.20 L93.99 18.26 L93.99 24.47 L87.16 28.41 Z", paint: "cubo2Dir" },
  { kind: "fill", d: "M69.07 26.53 L77.14 31.19 L69.07 35.84 L61.01 31.19 Z", paint: "cubo3Topo" },
  { kind: "fill", d: "M61.01 31.19 L69.07 35.84 L69.07 43.08 L61.01 38.42 Z", paint: "cubo3Esq" },
  { kind: "fill", d: "M69.07 35.84 L77.14 31.19 L77.14 38.42 L69.07 43.08 Z", paint: "cubo3Dir" },
];

/** As tintas do anel — o app troca por currentColor para seguir o tema. */
export const RING_PAINTS = ["prata", "prataBaixo"] as const;
