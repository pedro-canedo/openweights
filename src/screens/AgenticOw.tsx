// AgenticOw — o agente de código do OpenWeights, com tela própria.
//
// É o fork do DeepSeek Harness (pedro-canedo/agenticow) rodando como runtime
// verificado e aparecendo DENTRO da janela: a interface dele é uma webview
// filha, posicionada sobre a área de conteúdo desta tela. Aqui fica o que é do
// app — preparar, abrir, parar, remover — e o palco que a webview ocupa.
//
// A primeira abertura da sessão sobe o AgenticOw sozinha (baixando e
// verificando o runtime na primeira vez, com o progresso à vista). Parar é
// escolha da pessoa: voltar à tela depois disso não religa nada.

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import {
  agenticowStore,
  definirIdioma,
  desinstalar,
  esconder,
  iniciar,
  mostrar,
  parar,
  posicionar,
  refreshStatus,
  type Area,
} from "../lib/agenticow";
import { formatBytes, formatEta } from "../lib/format";
import { Card, Page, StatusDot } from "../components/ui/Shell";
import Icon, { type IconName } from "../components/ui/Icon";
import { navigate } from "../lib/nav";
import { startServer } from "../lib/api";
import { errorMessage } from "../lib/serverSession";

const botao =
  "rounded-lg border border-edge px-3 py-2 text-sm text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-50";
const botaoPrimario =
  "rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white disabled:opacity-50";

/** A subida automática acontece uma vez por sessão do app. */
let autoTentado = false;

export default function AgenticOw() {
  const { i18n } = useTranslation();
  const s = useSyncExternalStore(agenticowStore.subscribe, agenticowStore.get);

  useEffect(() => {
    void refreshStatus().then((st) => {
      if (!st || !st.supported || st.ready || autoTentado) return;
      if (agenticowStore.get().busy !== null) return;
      autoTentado = true;
      void iniciar(i18n.language);
    });
    // Só na montagem: o idioma tem o efeito dele logo abaixo.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // O AgenticOw fala o idioma do app, e acompanha a troca sem reiniciar.
  useEffect(() => {
    void definirIdioma(i18n.language);
  }, [i18n.language]);

  if (!s.status?.ready) return <Controle />;
  // No ar mas sem modelo nenhum: a interface do AgenticOw só mostraria um
  // seletor vazio. A pessoa escolhe aqui de qual fonte do OpenWeights vem o
  // cérebro — e a interface volta assim que o primeiro modelo chega.
  return s.status.models === 0 ? <Cerebro /> : <Palco />;
}

/** Algum modal do app aberto? A webview nativa ficaria por cima dele. */
function haModalAberto(): boolean {
  return document.querySelector('[role="dialog"], [aria-modal="true"]') !== null;
}

/** No ar: a interface do AgenticOw ocupa a tela, com uma barra fina em cima. */
function Palco() {
  const palco = useRef<HTMLDivElement>(null);
  const [erro, setErro] = useState<string | null>(null);

  useEffect(() => {
    const el = palco.current;
    if (!el) return;
    const area = (): Area => {
      const r = el.getBoundingClientRect();
      return { x: r.left, y: r.top, width: r.width, height: r.height };
    };
    let visivel = false;
    let encoberta = haModalAberto();
    const aplicar = () => {
      if (encoberta) return;
      if (visivel) {
        void posicionar(area());
        return;
      }
      visivel = true;
      mostrar(area()).catch((e: unknown) => {
        visivel = false;
        setErro(String(e));
      });
    };
    aplicar();
    const ro = new ResizeObserver(aplicar);
    ro.observe(el);
    window.addEventListener("resize", aplicar);
    // A webview é nativa: um modal do app, desenhado na webview de baixo,
    // ficaria atrás dela. Enquanto houver um aberto, ela sai da frente.
    const mo = new MutationObserver(() => {
      const modal = haModalAberto();
      if (modal && !encoberta) {
        encoberta = true;
        visivel = false;
        void esconder();
      } else if (!modal && encoberta) {
        encoberta = false;
        aplicar();
      }
    });
    mo.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["role", "aria-modal"],
    });
    return () => {
      ro.disconnect();
      mo.disconnect();
      window.removeEventListener("resize", aplicar);
      // Esconder, não fechar: a sessão aberta continua viva para a volta.
      void esconder();
    };
  }, []);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <Barra erro={erro} />
      {/* O lugar da webview do AgenticOw: vazio aqui, ocupado por ela. */}
      <div ref={palco} className="min-h-0 flex-1 bg-panel2" />
    </div>
  );
}

/** A barra fina de cima enquanto o AgenticOw está no ar. */
function Barra({ erro }: { erro: string | null }) {
  const { t, i18n } = useTranslation();
  const s = useSyncExternalStore(agenticowStore.subscribe, agenticowStore.get);
  return (
    <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-edge bg-panel px-4 py-2">
      <span className="text-sm font-medium">{t("agenticow.title")}</span>
      <span className="rounded-full border border-ok/40 bg-ok/10 px-2 py-0.5 text-[10px] text-ok">
        {t("agenticow.running")}
      </span>
      {s.catalogError !== null && (
        <span
          className="rounded-full border border-warn/40 bg-warn/10 px-2 py-0.5 text-[10px] text-warn"
          title={s.catalogError}
        >
          {t("agenticow.catalogError")}
        </span>
      )}
      {erro && <span className="truncate text-[11px] text-bad">{erro}</span>}
      <div className="ml-auto flex flex-wrap gap-2">
        <button
          onClick={() => void parar().then((ok) => ok && iniciar(i18n.language))}
          disabled={s.busy !== null}
          className={botao}
        >
          {t("agenticow.restart")}
        </button>
        <button onClick={() => void parar()} disabled={s.busy !== null} className={botao}>
          {s.busy === "stop" ? t("common.loading") : t("agenticow.stop")}
        </button>
      </div>
    </div>
  );
}

/**
 * No ar, sem modelo: o cérebro do AgenticOw vem sempre do OpenWeights, e aqui a
 * pessoa vê o que cada fonte tem e o próximo passo de cada uma. Nada se
 * configura no AgenticOw — as ações levam às telas do app (ou sobem o Servidor
 * Local direto, quando já há modelo na biblioteca).
 */
function Cerebro() {
  const { t } = useTranslation();
  const s = useSyncExternalStore(agenticowStore.subscribe, agenticowStore.get);
  const f = s.status?.sources;
  const [subindo, setSubindo] = useState(false);
  const [erro, setErro] = useState<string | null>(null);

  // A webview do AgenticOw sai da frente enquanto este painel está à vista.
  useEffect(() => {
    void esconder();
  }, []);

  const subirServidor = async () => {
    setSubindo(true);
    setErro(null);
    try {
      await startServer();
      // O catálogo chega sozinho (o motor subindo reenvia); o status vem junto.
      await refreshStatus();
    } catch (e) {
      setErro(errorMessage(e));
    } finally {
      setSubindo(false);
    }
  };

  const fontes = (tab: "openrouter" | "9router") => navigate("providers", { providersTab: tab });

  let local: Fonte;
  if (!f || f.localModels === 0) {
    local = {
      estado: t("agenticow.brain.local.none"),
      acao: t("agenticow.brain.local.find"),
      aoClicar: () => navigate("discover"),
    };
  } else if (!f.serverRunning) {
    local = {
      estado: t("agenticow.brain.local.stopped", { count: f.localModels }),
      acao: subindo ? t("common.loading") : t("agenticow.brain.local.start"),
      aoClicar: () => void subirServidor(),
      primaria: true,
      ocupada: subindo,
    };
  } else {
    local = {
      estado: t("agenticow.brain.local.running"),
      acao: t("agenticow.brain.local.open"),
      aoClicar: () => navigate("server"),
    };
  }

  const openrouter: Fonte = !f?.openrouterKey
    ? { estado: t("agenticow.brain.openrouter.noKey"), acao: t("agenticow.brain.configure"), aoClicar: () => fontes("openrouter") }
    : f.openrouterFavorites === 0
      ? { estado: t("agenticow.brain.openrouter.noFavorites"), acao: t("agenticow.brain.openrouter.pick"), aoClicar: () => fontes("openrouter") }
      : { estado: t("agenticow.brain.openrouter.ready", { count: f.openrouterFavorites }), acao: t("agenticow.brain.configure"), aoClicar: () => fontes("openrouter") };

  const nove: Fonte = !f?.ninerouterInstalled
    ? { estado: t("agenticow.brain.ninerouter.notInstalled"), acao: t("agenticow.brain.ninerouter.install"), aoClicar: () => fontes("9router") }
    : !f.ninerouterRunning
      ? { estado: t("agenticow.brain.ninerouter.stopped"), acao: t("agenticow.brain.configure"), aoClicar: () => fontes("9router") }
      : { estado: t("agenticow.brain.ninerouter.noModels"), acao: t("agenticow.brain.configure"), aoClicar: () => fontes("9router") };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <Barra erro={null} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-6 py-10">
          <h2 className="text-xl font-semibold">{t("agenticow.brain.title")}</h2>
          <p className="mt-2 max-w-2xl text-sm leading-relaxed text-dim">{t("agenticow.brain.body")}</p>
          <div className="mt-6 space-y-3">
            <CartaoFonte
              icone="cpu"
              nome={t("agenticow.brain.local.name")}
              descricao={t("agenticow.brain.local.hint")}
              fonte={local}
            />
            <CartaoFonte
              icone="network"
              nome={t("agenticow.brain.openrouter.name")}
              descricao={t("agenticow.brain.openrouter.hint")}
              fonte={openrouter}
            />
            <CartaoFonte
              icone="layers"
              nome={t("agenticow.brain.ninerouter.name")}
              descricao={t("agenticow.brain.ninerouter.hint")}
              fonte={nove}
            />
          </div>
          {erro && <p className="mt-4 text-sm text-bad">{erro}</p>}
          <p className="mt-6 text-xs leading-relaxed text-dim">{t("agenticow.brain.footnote")}</p>
        </div>
      </div>
    </div>
  );
}

interface Fonte {
  estado: string;
  acao: string;
  aoClicar: () => void;
  primaria?: boolean;
  ocupada?: boolean;
}

function CartaoFonte({
  icone,
  nome,
  descricao,
  fonte,
}: {
  icone: IconName;
  nome: string;
  descricao: string;
  fonte: Fonte;
}) {
  return (
    <div className="flex flex-wrap items-center gap-4 rounded-2xl border border-edge bg-panel px-5 py-4">
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-panel2 text-accent">
        <Icon name={icone} className="h-4 w-4" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium">{nome}</div>
        <div className="mt-0.5 text-xs leading-relaxed text-dim">{descricao}</div>
        <div className="mt-1.5 text-[12px] text-ink">{fonte.estado}</div>
      </div>
      <button
        onClick={fonte.aoClicar}
        disabled={fonte.ocupada}
        className={fonte.primaria ? botaoPrimario : botao}
      >
        {fonte.acao}
      </button>
    </div>
  );
}

/** Fora do ar: preparar e abrir, com o progresso à vista; e remover. */
function Controle() {
  const { t, i18n } = useTranslation();
  const s = useSyncExternalStore(agenticowStore.subscribe, agenticowStore.get);
  const [confirmando, setConfirmando] = useState(false);
  const logRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    logRef.current?.scrollTo(0, logRef.current.scrollHeight);
  }, [s.log]);

  const st = s.status;
  const suportado = st?.supported ?? true;
  const instalado = st?.installed ?? false;
  const erro = s.error ?? st?.lastError ?? null;
  const baixando = s.progress !== null && s.progress.totalBytes > 0;

  return (
    <div className="h-full overflow-y-auto">
      <Page icon="terminal" title={t("agenticow.title")} subtitle={t("agenticow.subtitle")}>
        <div className="harness-layout">
          <Card className="harness-launch" tone={erro ? "warn" : "normal"}>
            <div className="mb-6 flex items-center gap-3 text-accent">
              <Icon name="terminal" className="h-8 w-8" />
              <h2 className="harness-lead text-xl font-semibold text-ink">{t("agenticow.lead")}</h2>
            </div>
            <div className="flex flex-wrap items-start justify-between gap-4">
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <StatusDot tone={!suportado ? "off" : instalado ? "ok" : "off"} pulse={s.busy !== null} />
                  <span className="text-sm font-medium">
                    {!suportado
                      ? t("agenticow.unsupported")
                      : instalado
                        ? t("agenticow.stateInstalled")
                        : t("agenticow.stateNotInstalled")}
                  </span>
                </div>
                <p className="mt-1 max-w-xl text-[12px] leading-relaxed text-dim">
                  {!suportado
                    ? t("agenticow.unsupportedHint")
                    : instalado
                      ? t("agenticow.readyHint")
                      : t("agenticow.firstRunHint")}
                </p>
              </div>
              {suportado && (
                <div className="flex shrink-0 flex-wrap gap-2">
                  <button
                    onClick={() => void iniciar(i18n.language)}
                    disabled={s.busy !== null}
                    className={botaoPrimario}
                  >
                    {s.busy === "start"
                      ? t("common.loading")
                      : erro
                        ? t("agenticow.retry")
                        : t("agenticow.start")}
                  </button>
                </div>
              )}
            </div>

            {/* O download tem tamanho conhecido e ganha barra de verdade; as
                outras fases têm nome e cronômetro — uma barra que avançasse
                sozinha mentiria. */}
            {s.busy === "start" && baixando && s.progress && (
              <div className="mt-4">
                <div className="h-1.5 w-full overflow-hidden rounded-full bg-panel2">
                  <div
                    className="brand-gradient h-full rounded-full transition-[width]"
                    style={{ width: `${(s.progress.receivedBytes / s.progress.totalBytes) * 100}%` }}
                  />
                </div>
                <div className="mt-1 text-[11px] text-dim">
                  {formatBytes(s.progress.receivedBytes)} / {formatBytes(s.progress.totalBytes)}
                  {s.phase ? ` — ${t(`agenticow.phase.${s.phase}`, { defaultValue: s.phase })}` : ""}
                </div>
              </div>
            )}

            {s.busy && !(s.busy === "start" && baixando) && (
              <div className="mt-4">
                <div className="h-1.5 w-full overflow-hidden rounded-full bg-panel2">
                  <div className="brand-gradient h-full w-1/3 animate-pulse rounded-full" />
                </div>
                <div className="mt-1 text-[11px] text-dim">
                  {s.busy === "start" && s.phase
                    ? t(`agenticow.phase.${s.phase}`, { defaultValue: s.phase })
                    : t(`agenticow.working.${s.busy}`)}{" "}
                  · {formatEta(s.segundos)}
                </div>
              </div>
            )}

            {s.busy === "start" && (
              <p className="mt-2 text-[11px] leading-relaxed text-dim">{t("agenticow.startHint")}</p>
            )}

            {s.log.length > 0 && (
              <details className="mt-3">
                <summary className="cursor-pointer text-[11px] text-dim">{t("agenticow.log")}</summary>
                <pre
                  ref={logRef}
                  className="mt-2 max-h-48 overflow-y-auto rounded-lg border border-edge bg-panel2 p-2 text-[10px] leading-relaxed text-dim"
                >
                  {s.log.join("\n")}
                </pre>
              </details>
            )}

            {erro && (
              <p className="mt-3 rounded-lg border border-bad/40 bg-bad/10 px-3 py-2 text-[12px] text-bad">
                {t("agenticow.error")}: {erro}
              </p>
            )}
          </Card>

          <Card title={t("agenticow.aboutTitle")} hint={t("agenticow.aboutBody")}>
            <dl className="mt-3 grid grid-cols-2 gap-x-8 gap-y-1.5 text-[12px]">
              <div>
                <dt className="text-dim">{t("agenticow.runtime")}</dt>
                <dd className="truncate" title={st?.revision}>
                  {st ? `${st.tag.replace(/^agenticow-runtime-/, "")}` : "—"}
                </dd>
              </div>
              <div>
                <dt className="text-dim">{t("agenticow.base")}</dt>
                <dd>{st?.upstreamTag ?? t("agenticow.baseUnknown")}</dd>
              </div>
            </dl>
            <ul className="mt-4 space-y-3 text-[12px] leading-relaxed text-dim">
              {["controlIsolated", "controlProviders", "controlKeys", "controlPrivacy"].map((key) => (
                <li key={key} className="flex gap-3">
                  <Icon name="check" className="mt-0.5 h-4 w-4 shrink-0 text-accent" />
                  {t(`agenticow.${key}`)}
                </li>
              ))}
            </ul>

            {/* Remoção em dois passos, como o descarte de modelo. */}
            {instalado && (
              <div className="mt-4 border-t border-edge pt-3">
                {!confirmando ? (
                  <button
                    onClick={() => setConfirmando(true)}
                    disabled={s.busy !== null}
                    className="rounded-lg border border-edge px-3 py-2 text-sm text-dim hover:border-bad/60 hover:text-bad disabled:opacity-50"
                  >
                    {t("agenticow.uninstall")}
                  </button>
                ) : (
                  <div className="rounded-lg border border-bad/40 bg-bad/10 px-3 py-2">
                    <p className="text-[12px] text-bad">{t("agenticow.uninstallWarning")}</p>
                    <div className="mt-2 flex flex-wrap gap-2">
                      <button
                        onClick={() => {
                          setConfirmando(false);
                          void desinstalar(true);
                        }}
                        className="rounded-lg bg-bad px-3 py-1.5 text-[12px] font-medium text-white"
                      >
                        {t("agenticow.uninstallAll")}
                      </button>
                      <button
                        onClick={() => {
                          setConfirmando(false);
                          void desinstalar(false);
                        }}
                        className={botao}
                      >
                        {t("agenticow.uninstallKeepData")}
                      </button>
                      <button onClick={() => setConfirmando(false)} className={botao}>
                        {t("common.cancel")}
                      </button>
                    </div>
                  </div>
                )}
              </div>
            )}
          </Card>
        </div>
      </Page>
    </div>
  );
}
