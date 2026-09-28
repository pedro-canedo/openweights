import i18next from "i18next";

/**
 * Um número com `casas` decimais, no formato do idioma da interface: "1,5"
 * em português, "1.5" em inglês. O idioma é lido na hora (o singleton do
 * i18next), então trocar de idioma vale na próxima pintura.
 */
export function formatNumber(n: number, casas = 0): string {
  return n.toLocaleString(i18next.language || "pt-BR", {
    minimumFractionDigits: casas,
    maximumFractionDigits: casas,
  });
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  const gib = bytes / 2 ** 30;
  if (gib >= 1) return `${formatNumber(gib, gib >= 10 ? 0 : 1)} GB`;
  const mib = bytes / 2 ** 20;
  if (mib >= 1) return `${formatNumber(mib)} MB`;
  return `${formatNumber(bytes / 1024)} KB`;
}

export function formatCount(n: number): string {
  if (n >= 1_000_000) return `${formatNumber(n / 1_000_000, 1)}M`;
  if (n >= 1_000) return `${formatNumber(n / 1_000, 1)}k`;
  return `${n}`;
}

export function formatParams(n: number | null): string {
  if (n == null) return "—";
  if (n >= 1e12) return `${formatNumber(n / 1e12, 1)}T`;
  if (n >= 1e9) return `${formatNumber(n / 1e9, 1)}B`;
  return `${formatNumber(n / 1e6)}M`;
}

export function downloadPercent(received: number, total: number): number {
  if (total <= 0) return 0;
  return Math.max(0, Math.min(100, (received / total) * 100));
}

/** "há 2 horas" / "in 3 days" — do próprio navegador, sem biblioteca. `short`: "há 2 h". */
export function formatAgo(
  lang: string,
  tsMs: number,
  style: Intl.RelativeTimeFormatStyle = "long",
): string {
  const rtf = new Intl.RelativeTimeFormat(lang, { numeric: "auto", style });
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

/**
 * Duração em milissegundos, como uma pessoa a lê.
 *
 * Formatador ÚNICO de duração da interface: abaixo de um segundo mostra o
 * sub-segundo (uma ferramenta que respondeu em 90 ms), abaixo de um minuto
 * mostra segundos inteiros, e daí para cima entra em minutos — "Pensou por
 * 173.9s" não responde a pergunta que a pessoa tem, "2min 54s" responde.
 */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "—";
  if (ms < 1_000) return `${Math.round(ms)}ms`;
  return formatEta(ms / 1000);
}
