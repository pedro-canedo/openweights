// O vocabulário que o app usa porque é o certo, e que quem começa não tem onde
// perguntar. Cada termo tem nome e explicação em `glossary.<id>.*` (nos dois
// idiomas); `<Termo id="…" />` põe o "?" ao lado da primeira ocorrência.

export const TERMOS = [
  "quantization",
  "gguf",
  "context",
  "vram",
  "tokps",
  "moe",
  "kvcache",
  "slot",
] as const;

export type TermoId = (typeof TERMOS)[number];
