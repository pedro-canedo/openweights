// Card "Estatísticas de serviço": o que o servidor local já atendeu — tokens
// processados, reaproveitados do cache e gerados, com as velocidades médias
// de processamento de prompt e de geração.
//
// Os números vêm dos counters do próprio motor (/metrics), então cobrem TODO
// o tráfego servido: chat interno e apps externos (Claude Code,
// Cursor…). "Sessão" = desde que o app abriu; "Desde sempre" = acumulado no
// banco. As velocidades são médias do recorte escolhido, não instantâneas.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { serveStats, serveStatsClear } from "../../lib/api";
import { routerModels } from "../../lib/flags";
import type { ServeAgg, ServeStatsDto } from "../../lib/types";
import { Chips, Select } from "../form/controls";

type Scope = "session" | "allTime";

export default function ServeStatsCard({ running }: { running: boolean }) {
  const { t, i18n } = useTranslation();
  const [scope, setScope] = useState<Scope>("session");
  // "" = todos os modelos (o backend agrega tudo quando model = null).
  const [model, setModel] = useState("");
  const [dto, setDto] = useState<ServeStatsDto | null>(null);
  const [loaded, setLoaded] = useState<string[]>([]);
  const [clearArmed, setClearArmed] = useState(false);
  const [clearing, setClearing] = useState(false);
  const disarmTimer = useRef<number | null>(null);

  // Polling de 10 s com a tela aberta (a própria chamada dispara uma coleta
  // no backend). Refaz na hora ao trocar o filtro ou quando o servidor
  // sobe/desce; o cleanup para o relógio no unmount.
  useEffect(() => {
    let alive = true;
    const poll = () => {
      serveStats(model || null)
        .then((d) => {
          if (alive) setDto(d);
        })
        .catch(() => {
          // sem resposta agora — o valor anterior continua na tela
        });
      // Modelos carregados entram no Select mesmo antes de terem dados.
      if (running) {
        routerModels()
          .then((list) => {
            if (alive)
              setLoaded(
                list.filter((m) => m.state === "loaded").map((m) => m.id),
              );
          })
          .catch(() => {});
      }
    };
    if (!running) setLoaded([]);
    poll();
    const timer = setInterval(poll, 10_000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [model, running]);

  // O timer da confirmação leve não pode sobreviver ao card.
  useEffect(
    () => () => {
      if (disarmTimer.current != null) window.clearTimeout(disarmTimer.current);
    },
    [],
  );

  // Confirmação leve do Limpar: o primeiro clique arma ("Confirmar?"), o
  // segundo executa; sem o segundo clique em 4 s, desarma sozinho.
  async function limpar() {
    if (!clearArmed) {
      setClearArmed(true);
      if (disarmTimer.current != null) window.clearTimeout(disarmTimer.current);
      disarmTimer.current = window.setTimeout(() => setClearArmed(false), 4000);
      return;
    }
    if (disarmTimer.current != null) window.clearTimeout(disarmTimer.current);
    setClearArmed(false);
    setClearing(true);
    try {
      await serveStatsClear();
      setDto(await serveStats(null));
      setModel("");
    } catch {
      // sem resposta agora — o próximo poll conta a história
    } finally {
      setClearing(false);
    }
  }

  // União: modelos com dados (do DTO) + carregados no Router agora.
  const models = [...new Set([...(dto?.models ?? []), ...loaded])].sort();

  const agg: ServeAgg | null = dto
    ? scope === "session"
      ? dto.session
      : dto.allTime
    : null;
  const vazio = agg != null && agg.totalTokens <= 0;

  // Números seguem o idioma do app (separador de milhar pt/en), não o do SO.
  const fmtInt = (n: number) =>
    new Intl.NumberFormat(i18n.language).format(Math.round(n));
  const fmtTps = (n: number | null) =>
    n == null
      ? "—"
      : `${new Intl.NumberFormat(i18n.language, {
          minimumFractionDigits: 1,
          maximumFractionDigits: 1,
        }).format(n)} tok/s`;
  const fmtPct = (x: number | null) =>
    x == null
      ? "—"
      : new Intl.NumberFormat(i18n.language, {
          style: "percent",
          maximumFractionDigits: 1,
        }).format(x);

  const btn =
    "rounded-lg border border-edge px-2.5 py-1.5 text-xs text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40";

  return (
    <div className="mt-4 rounded-xl border border-edge bg-panel p-5">
      <div className="flex flex-wrap items-center gap-3">
        <div className="text-sm font-medium">{t("server.stats.title")}</div>
        <div className="ml-auto flex flex-wrap items-center gap-2">
          <Chips
            value={scope}
            onChange={setScope}
            options={[
              { id: "session", label: t("server.stats.session") },
              { id: "allTime", label: t("server.stats.allTime") },
            ]}
          />
          <Select
            value={model}
            onChange={setModel}
            className="max-w-56 text-xs"
            options={[
              { value: "", label: t("server.stats.allModels") },
              ...models.map((id) => ({ value: id, label: id })),
            ]}
          />
          <button
            type="button"
            disabled={clearing || dto == null}
            onClick={() => void limpar()}
            className={`${btn} ${clearArmed ? "border-bad text-bad hover:border-bad hover:text-bad" : ""}`}
          >
            {clearing
              ? t("common.loading")
              : clearArmed
                ? t("server.stats.clearConfirm")
                : t("server.stats.clear")}
          </button>
        </div>
      </div>

      {/* Servidor parado: os números continuam valendo, mas são históricos. */}
      {dto != null && !dto.running && (
        <p className="mt-2 text-[11px] leading-relaxed text-warn">
          {t("server.stats.serverDown")}
        </p>
      )}

      {dto == null && (
        <p className="mt-3 text-[11px] text-dim">{t("common.loading")}</p>
      )}

      {agg != null && vazio && (
        <p className="mt-3 text-[11px] leading-relaxed text-dim">
          {t("server.stats.empty")}
        </p>
      )}

      {agg != null && !vazio && (
        <>
          <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3">
            <Tile
              label={t("server.stats.totalTokens")}
              value={fmtInt(agg.totalTokens)}
            />
            <Tile
              label={t("server.stats.cachedTokens")}
              value={fmtInt(agg.cachedTokens)}
            />
            <Tile
              label={t("server.stats.cacheEfficiency")}
              value={fmtPct(agg.cacheEfficiency)}
            />
          </div>
          <div className="mt-2 grid grid-cols-2 gap-2">
            <Tile
              label={t("server.stats.promptSpeed")}
              value={fmtTps(agg.avgPromptTps)}
              hint={t("server.stats.avgHint")}
            />
            <Tile
              label={t("server.stats.genSpeed")}
              value={fmtTps(agg.avgGenTps)}
              hint={t("server.stats.avgHint")}
            />
          </div>
        </>
      )}

      <p className="mt-3 text-[11px] leading-relaxed text-dim">
        {t("server.stats.allClients")}
      </p>
    </div>
  );
}

/** Tile no estilo dataviz do app (mesma receita do StatTile da Atividade). */
function Tile({
  label,
  value,
  hint,
}: {
  label: string;
  /** Já formatado — ou "—" quando o número não existe. */
  value: string;
  hint?: string;
}) {
  return (
    <div className="rounded-xl border border-edge bg-panel2 px-3 py-2.5">
      <div className="text-[11px] text-dim">{label}</div>
      <div className="mt-0.5 text-lg leading-tight tabular-nums">{value}</div>
      {hint && <div className="mt-0.5 text-[11px] text-dim">{hint}</div>}
    </div>
  );
}
