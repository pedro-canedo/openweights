// O painel de logs: tudo que roda por baixo do app, acoplado acima da barra de
// status. Filtra por origem e por texto, pausa (para ler sem a lista andar),
// copia, salva e limpa.

import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, IconButton } from "./ui/Button";
import { toast } from "./ui/Toast";
import { salvarExportacao, mostrarExportacao } from "../lib/exportar";
import {
  ORIGENS,
  filtrarLogs,
  horaDoLog,
  iniciarLogs,
  limparLogs,
  painelDeLogs,
  textoDosLogs,
  useLogs,
  type LinhaDeLog,
} from "../lib/logs";

const COR: Record<string, string> = {
  servidor: "text-accent-ink",
  decisor: "text-ok",
  "9router": "text-warn",
  gateway: "text-accent-ink",
  app: "text-dim",
};

export default function LogDrawer() {
  const { t } = useTranslation();
  const todas = useLogs();
  const [origens, setOrigens] = useState<ReadonlySet<string>>(() => new Set(ORIGENS));
  const [busca, setBusca] = useState("");
  const [pausado, setPausado] = useState(false);
  // Pausar congela o que se vê; as linhas novas continuam chegando no store.
  const congeladas = useRef<readonly LinhaDeLog[]>([]);
  if (!pausado) congeladas.current = todas;
  const base = pausado ? congeladas.current : todas;
  const visiveis = useMemo(() => filtrarLogs(base, origens, busca), [base, origens, busca]);

  useEffect(() => {
    void iniciarLogs();
  }, []);

  // Segue a última linha, salvo pausado ou com a pessoa lendo mais acima.
  const rolagem = useRef<HTMLDivElement>(null);
  const noFim = useRef(true);
  useEffect(() => {
    const el = rolagem.current;
    if (el && noFim.current && !pausado) el.scrollTop = el.scrollHeight;
  }, [visiveis, pausado]);

  const alternar = (o: string) =>
    setOrigens((atual) => {
      const novo = new Set(atual);
      if (novo.has(o)) novo.delete(o);
      else novo.add(o);
      return novo;
    });

  const copiar = () => {
    void navigator.clipboard
      .writeText(textoDosLogs(visiveis))
      .then(() => toast({ tone: "ok", message: t("logs.copied", { n: visiveis.length }), duration: 2500 }))
      .catch((e) => toast({ tone: "bad", message: t("logs.saveFailed", { error: String(e) }), duration: 6000 }));
  };
  const salvar = async () => {
    try {
      const nome = `openweights-logs-${new Date().toISOString().replace(/[:.]/g, "-")}.txt`;
      const caminho = await salvarExportacao(nome, textoDosLogs(visiveis));
      toast({
        tone: "ok",
        message: t("logs.saved", { path: caminho }),
        action: { label: t("logs.showFolder"), run: () => void mostrarExportacao(caminho) },
        duration: 8000,
      });
    } catch (e) {
      toast({ tone: "bad", message: t("logs.saveFailed", { error: String(e) }), duration: 0 });
    }
  };

  return (
    <section
      aria-label={t("logs.title")}
      className="flex h-64 shrink-0 flex-col border-t border-edge bg-panel"
    >
      <div className="flex flex-wrap items-center gap-2 border-b border-edge px-3 py-1.5">
        <h2 className="text-[12px] font-medium text-ink">{t("logs.title")}</h2>
        <div role="group" aria-label={t("logs.sources")} className="flex flex-wrap gap-1">
          {ORIGENS.map((o) => (
            <button
              key={o}
              type="button"
              aria-pressed={origens.has(o)}
              onClick={() => alternar(o)}
              className={`rounded-full border px-2 py-0.5 text-[11px] transition-colors ${
                origens.has(o)
                  ? "border-accent/60 bg-accent/10 text-ink"
                  : "border-edge text-dim hover:text-ink"
              }`}
            >
              {t(`logs.source.${o}`)}
            </button>
          ))}
        </div>
        <input
          value={busca}
          onChange={(e) => setBusca(e.target.value)}
          aria-label={t("logs.filter")}
          placeholder={t("logs.filter")}
          className="min-w-32 flex-1 rounded-lg border border-edge bg-panel2 px-2 py-1 text-[12px] outline-none focus-visible:border-accent"
        />
        <span className="text-[11px] tabular-nums text-dim" data-testid="log-count">
          {t("logs.count", { count: visiveis.length })}
        </span>
        <Button size="sm" aria-pressed={pausado} onClick={() => setPausado((p) => !p)}>
          {pausado ? t("logs.resume") : t("logs.pause")}
        </Button>
        <Button size="sm" onClick={copiar} disabled={visiveis.length === 0}>
          {t("logs.copy")}
        </Button>
        <Button size="sm" onClick={() => void salvar()} disabled={visiveis.length === 0}>
          {t("logs.save")}
        </Button>
        <Button size="sm" variant="ghost" onClick={() => void limparLogs()}>
          {t("logs.clear")}
        </Button>
        <IconButton
          icon="close"
          size="sm"
          label={t("logs.close")}
          onClick={() => painelDeLogs.definir(false)}
        />
      </div>
      <div
        ref={rolagem}
        tabIndex={0}
        role="log"
        aria-live="off"
        aria-label={t("logs.lines")}
        onScroll={(e) => {
          const el = e.currentTarget;
          noFim.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
        className="min-h-0 flex-1 overflow-y-auto px-3 py-2 font-mono text-[11px] leading-relaxed select-text"
      >
        {visiveis.length === 0 ? (
          <p className="text-dim">{todas.length === 0 ? t("logs.empty") : t("logs.noMatch")}</p>
        ) : (
          visiveis.map((l) => (
            <div key={l.seq} className="whitespace-pre-wrap break-words">
              <span className="text-dim">{horaDoLog(l.ts)} </span>
              <span className={COR[l.origem] ?? "text-dim"}>[{l.origem}] </span>
              <span className="text-ink">{l.texto}</span>
            </div>
          ))
        )}
      </div>
    </section>
  );
}
