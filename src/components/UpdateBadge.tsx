// Versão instalada no rodapé da barra lateral — e, quando existe versão
// nova, o botão que atualiza.
//
// Fica onde a versão já ficava de propósito: é o lugar onde a pessoa olha
// para saber em que versão está, então é onde ela espera descobrir que há
// outra. Enquanto não há novidade (o caso comum) o componente é a mesma
// linha discreta de antes.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  checkForUpdate,
  getAppVersion,
  installUpdate,
  onUpdateProgress,
  type UpdateInfo,
} from "../lib/api";

type Fase = "parado" | "baixando" | "erro";

export default function UpdateBadge() {
  const { t } = useTranslation();
  const [versao, setVersao] = useState<string | null>(null);
  const [nova, setNova] = useState<UpdateInfo | null>(null);
  const [fase, setFase] = useState<Fase>("parado");
  const [pct, setPct] = useState<number | null>(null);
  const [erro, setErro] = useState<string | null>(null);
  const vivo = useRef(true);

  useEffect(() => {
    vivo.current = true;
    getAppVersion()
      .then((v) => vivo.current && setVersao(v))
      .catch(() => {});
    // Falha ao consultar não vira aviso na tela: sem internet, o normal é
    // não saber — e um erro no rodapé a cada abertura seria só ruído.
    checkForUpdate()
      .then((u) => vivo.current && setNova(u))
      .catch(() => {});
    return () => {
      vivo.current = false;
    };
  }, []);

  useEffect(() => {
    if (fase !== "baixando") return;
    let cancelar: (() => void) | undefined;
    onUpdateProgress(({ baixado, total }) => {
      if (!vivo.current) return;
      setPct(total && total > 0 ? Math.min(100, (baixado / total) * 100) : null);
    }).then((un) => {
      cancelar = un;
      if (!vivo.current) un();
    });
    return () => cancelar?.();
  }, [fase]);

  async function atualizar() {
    setFase("baixando");
    setErro(null);
    try {
      // Em caso de sucesso o app reinicia e nada abaixo roda.
      await installUpdate();
    } catch (e) {
      if (!vivo.current) return;
      setFase("erro");
      setErro(e instanceof Error ? e.message : String(e));
    }
  }

  const rotulo = versao ? `v${versao}` : "";

  if (!nova) {
    return (
      <div className="shrink-0 px-4 py-2.5 text-[11px] text-dim">{rotulo}</div>
    );
  }

  return (
    <div className="shrink-0 px-3 py-2.5">
      <button
        onClick={atualizar}
        disabled={fase === "baixando"}
        title={nova.notes ?? undefined}
        className="flex w-full items-center gap-2 rounded-lg border border-accent/40 bg-accent/10 px-2.5 py-1.5 text-left text-[11px] text-ink transition-colors hover:bg-accent/20 disabled:cursor-default disabled:opacity-70"
      >
        <svg
          className="h-3.5 w-3.5 shrink-0 text-accent"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          viewBox="0 0 24 24"
        >
          <path d="M12 19V5m0 0l-6 6m6-6l6 6" />
        </svg>
        <span className="min-w-0 flex-1 truncate">
          {fase === "baixando"
            ? pct === null
              ? t("update.downloading")
              : t("update.downloadingPct", { pct: Math.round(pct) })
            : t("update.available", { version: nova.version })}
        </span>
      </button>

      {fase === "baixando" && pct !== null && (
        <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-panel2">
          <div
            className="h-full bg-accent transition-[width]"
            style={{ width: `${pct}%` }}
          />
        </div>
      )}

      {fase === "erro" && (
        <p className="mt-1.5 px-0.5 text-[10px] leading-snug text-red-400">
          {t("update.failed")} {erro}
        </p>
      )}

      <div className="mt-1.5 px-0.5 text-[10px] text-dim">{rotulo}</div>
    </div>
  );
}
