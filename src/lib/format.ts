export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  const gib = bytes / 2 ** 30;
  if (gib >= 1) return `${gib.toFixed(gib >= 10 ? 0 : 1)} GB`;
  const mib = bytes / 2 ** 20;
  if (mib >= 1) return `${mib.toFixed(0)} MB`;
  return `${(bytes / 1024).toFixed(0)} KB`;
}

export function formatCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return `${n}`;
}

export function formatParams(n: number | null): string {
  if (n == null) return "—";
  if (n >= 1e12) return `${(n / 1e12).toFixed(1)}T`;
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  return `${(n / 1e6).toFixed(0)}M`;
}

export function downloadPercent(received: number, total: number): number {
  if (total <= 0) return 0;
  return Math.max(0, Math.min(100, (received / total) * 100));
}

/** "há 2 h" / "in 3 days" — do próprio navegador, sem biblioteca. */
export function formatAgo(lang: string, tsMs: number): string {
  const rtf = new Intl.RelativeTimeFormat(lang, { numeric: "auto" });
  const minutos = Math.round((tsMs - Date.now()) / 60_000);
  if (Math.abs(minutos) < 60) return rtf.format(minutos, "minute");
  const horas = Math.round(minutos / 60);
  if (Math.abs(horas) < 24) return rtf.format(horas, "hour");
  return rtf.format(Math.round(horas / 24), "day");
}

/** Tempo restante compacto (ex.: "3min 20s"). */
export function formatEta(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "—";
  const s = Math.round(seconds);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}min ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}min`;
}
