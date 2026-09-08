// O painel de detalhe da tela Descobrir: tudo o que se sabe sobre UM modelo.
//
// Antes isto era uma gaveta que deslizava por cima da lista e só mostrava as
// quantizações — para ler a descrição do modelo era preciso sair do app e
// abrir o Hub no navegador. Aqui o painel é fixo ao lado da lista e responde
// às três perguntas na ordem em que elas aparecem: *cabe na minha máquina?*
// (as quantizações, com o veredito de hardware), *o que é isto?* (os campos
// e o cartão do autor) e *e se eu quiser mais?* (o link para o Hub).
//
// O README vem como Markdown e é renderizado pelo mesmo componente do chat.
// Ele NÃO renderiza HTML embutido — muitos cartões trazem `<div>`s e imagens
// de terceiros, e um app desktop não deve executar marcação de estranho.

import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { ModelSummary, QuantsView, QuantView } from "../../lib/types";
import { getModelQuants, hfLogin, modelReadme, startDownload } from "../../lib/api";
import { formatAgo, formatBytes, formatCount, formatParams } from "../../lib/format";
import Markdown from "../chat/Markdown";
import AuthorAvatar from "./AuthorAvatar";
import { useHfAccess } from "../../lib/hfAccess";
import CapBadges from "./CapBadges";
import GateNotice from "./GateNotice";
import VerdictBadge from "./VerdictBadge";

async function abrirNoHub(repoId: string) {
  const url = `https://huggingface.co/${repoId}`;
  try {
    await openUrl(url);
  } catch {
    window.open(url, "_blank", "noopener");
  }
}

/** Um campo do bloco "Detalhes": rótulo pequeno, valor legível. */
function Campo({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-2">
      <span className="shrink-0 text-[11px] text-dim">{label}</span>
      <span className="min-w-0 truncate text-[12px] text-ink">{children}</span>
    </div>
  );
}

function QuantRow({
  quant,
  started,
  onDownload,
  ctxLen,
}: {
  quant: QuantView;
  started: boolean;
  onDownload: () => void;
  ctxLen: number;
}) {
  const { t } = useTranslation();

  return (
    <div
      className={`rounded-xl border p-3 transition-colors ${
        quant.recommended ? "border-accent/60 bg-accent/5" : "border-edge bg-panel2/40"
      }`}
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="rounded-md bg-panel2 px-1.5 py-0.5 font-mono text-[10px] text-dim">
          GGUF
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-[13px] font-medium text-ink">
          {quant.label}
        </span>
        <span className="shrink-0 text-xs tabular-nums text-dim">
          {formatBytes(quant.sizeBytes)}
        </span>
        <button
          onClick={onDownload}
          disabled={started}
          className={`shrink-0 rounded-lg px-3 py-1.5 text-xs font-medium transition-colors ${
            started
              ? "cursor-default bg-panel2 text-dim"
              : "bg-accent text-white hover:opacity-90"
          }`}
        >
          {started ? t("discover.downloading") : t("discover.download")}
        </button>
      </div>

      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        <VerdictBadge verdict={quant.verdict} />
        {quant.recommended && (
          <span className="rounded-full bg-accent/15 px-2 py-0.5 text-[11px] font-medium text-accent">
            {t("badge.recommended")}
          </span>
        )}
        {/* O arquivo é o que se baixa; o que precisa caber é isto. A
            diferença entre os dois números é o que faz alguém escolher
            errado. */}
        <span className="text-[11px] tabular-nums text-dim">
          {t("badge.needsMemory", {
            total: formatBytes(quant.estTotalBytes),
            ctx: (ctxLen / 1024).toFixed(0),
          })}
        </span>
      </div>

      {quant.verdict.kind === "partial" && (
        <p className="mt-1.5 text-[11px] text-warn/90">{t("badge.partialWarn")}</p>
      )}
    </div>
  );
}

export default function ModelDetail({ model }: { model: ModelSummary }) {
  const { t, i18n } = useTranslation();
  const [view, setView] = useState<QuantsView | null>(null);
  const [failed, setFailed] = useState(false);
  const [started, setStarted] = useState<ReadonlySet<string>>(new Set());
  const [readme, setReadme] = useState<string | null>(null);

  // Quantizações do repositório escolhido.
  useEffect(() => {
    let cancelado = false;
    setView(null);
    setFailed(false);
    setStarted(new Set());
    getModelQuants(model.id, model.paramsTotal)
      .then((q) => !cancelado && setView(q))
      .catch(() => !cancelado && setFailed(true));
    return () => {
      cancelado = true;
    };
  }, [model.id, model.paramsTotal]);

  // O cartão do autor. Falha em silêncio: um repositório sem README não é
  // erro, e a tela já tem o que mostrar sem ele.
  useEffect(() => {
    let cancelado = false;
    setReadme(null);
    modelReadme(model.id)
      .then((txt) => !cancelado && setReadme(txt))
      .catch(() => !cancelado && setReadme(""));
    return () => {
      cancelado = true;
    };
  }, [model.id]);

  // Menor bits primeiro (desconhecidos por último), desempatando pelo tamanho.
  const ordenadas = useMemo(
    () =>
      view
        ? [...view.quants].sort(
            (a, b) =>
              (a.bits ?? Number.MAX_SAFE_INTEGER) -
                (b.bits ?? Number.MAX_SAFE_INTEGER) || a.sizeBytes - b.sizeBytes,
          )
        : null,
    [view],
  );

  // O portão do repositório, e o que ele exige desta conta. Só é consultado
  // depois que as quantizações chegam, para a sondagem testar o arquivo que
  // a pessoa vai baixar de verdade.
  const gate = useHfAccess(
    model.id,
    ordenadas?.[0]?.files?.[0],
    model.gated && (view != null || failed),
  );
  /** A quantização que espera o portão abrir. */
  const [pendente, setPendente] = useState<QuantView | null>(null);
  const [entrando, setEntrando] = useState(false);

  // Entrar com a conta: o comando só volta quando a autorização acontece no
  // navegador (ou expira), e aí a resposta do portão já pode ter mudado.
  const entrar = useCallback(async () => {
    setEntrando(true);
    try {
      await hfLogin();
      gate.revalidar();
    } catch (err) {
      console.error(err);
    } finally {
      setEntrando(false);
    }
    // `revalidar` é estável; depender do objeto inteiro recriaria isto a
    // cada render sem necessidade.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gate.revalidar]);

  const iniciar = useCallback(
    (q: QuantView) => {
      // Feedback imediato; reverte se o backend recusar.
      setStarted((prev) => new Set(prev).add(q.artifactName));
      startDownload(model.id, q.artifactName).catch((err) => {
        console.error(err);
        setStarted((prev) => {
          const next = new Set(prev);
          next.delete(q.artifactName);
          return next;
        });
      });
    },
    [model.id],
  );

  // Clicar em baixar num modelo com o portão fechado não é erro nem recusa:
  // é o começo do caminho. O app abre a página onde se aceita, guarda a
  // quantização escolhida e passa a perguntar ao Hub se o portão abriu —
  // quando abre, o download que ficou parado começa sozinho.
  const baixar = (q: QuantView) => {
    if (gate.bloqueado) {
      setPendente(q);
      if (gate.report?.access === "noToken") void entrar();
      else {
        void abrirNoHub(model.id);
        gate.aguardarAceite();
      }
      return;
    }
    iniciar(q);
  };

  useEffect(() => {
    if (!pendente || !gate.liberado) return;
    const q = pendente;
    setPendente(null);
    iniciar(q);
  }, [pendente, gate.liberado, iniciar]);

  const atualizado = model.updatedAt
    ? formatAgo(i18n.language, new Date(model.updatedAt).getTime())
    : null;

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-3xl px-6 py-6">
        <header className="flex items-start gap-4">
          <AuthorAvatar author={model.author} size={56} />
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-lg font-semibold text-ink">
              {model.name}
            </h2>
            <p className="truncate text-[13px] text-dim">{model.id}</p>
          </div>
          <button
            onClick={() => void abrirNoHub(model.id)}
            className="shrink-0 rounded-lg border border-edge px-3 py-2 text-xs font-medium text-dim transition-colors hover:border-accent hover:text-ink"
          >
            {t("discover.openOnHf")} ↗
          </button>
        </header>

        <div className="mt-4 flex flex-wrap items-center gap-2">
          <span className="rounded-lg border border-edge bg-panel2/60 px-2.5 py-1 text-[11px] tabular-nums text-dim">
            ↓ {formatCount(model.downloads)}
          </span>
          <span className="rounded-lg border border-edge bg-panel2/60 px-2.5 py-1 text-[11px] tabular-nums text-dim">
            ♥ {formatCount(model.likes)}
          </span>
          {atualizado && (
            <span className="rounded-lg border border-edge bg-panel2/60 px-2.5 py-1 text-[11px] text-dim">
              {t("discover.updated", { when: atualizado })}
            </span>
          )}
          {model.license && (
            <span className="rounded-lg border border-edge bg-panel2/60 px-2.5 py-1 text-[11px] text-dim">
              {model.license}
            </span>
          )}
        </div>

        {/* ------------------------------------------- opções de download */}
        <section className="mt-6">
          <h3 className="text-sm font-medium text-ink">
            {t("discover.downloadOptions")}
          </h3>
          <p className="mt-0.5 text-[12px] text-dim">{t("discover.quantsHint")}</p>

          {/* O aviso só aparece depois que o Hub responde se ESTA conta pode
              baixar — e some quando pode. A sondagem espera as quantizações
              para testar o arquivo que vai ser baixado de verdade; se a lista
              falhar, o aviso sonda sozinho um arquivo comum a todo
              repositório. */}
          {model.gated && (view != null || failed) && (
            <GateNotice
              report={gate.report}
              checando={gate.checando}
              falhou={gate.falhou}
              aguardando={gate.aguardando}
              entrando={entrando}
              onOpenHub={() => {
                void abrirNoHub(model.id);
                gate.aguardarAceite();
              }}
              onRecheck={gate.revalidar}
              onLogin={() => void entrar()}
            />
          )}

          <div className="mt-3">
            {failed ? (
              <div className="rounded-xl border border-dashed border-edge p-8 text-center text-sm text-dim">
                {t("common.error")}
              </div>
            ) : ordenadas == null ? (
              <div className="flex flex-col gap-2">
                {Array.from({ length: 3 }).map((_, i) => (
                  <div
                    key={i}
                    className="animate-pulse rounded-xl border border-edge p-3"
                  >
                    <div className="h-4 w-1/3 rounded bg-panel2" />
                    <div className="mt-3 h-3 w-2/3 rounded bg-panel2" />
                  </div>
                ))}
              </div>
            ) : ordenadas.length === 0 ? (
              <div className="rounded-xl border border-dashed border-edge p-8 text-center text-sm text-dim">
                {t("discover.noQuants")}
              </div>
            ) : (
              <div className="flex flex-col gap-2">
                {ordenadas.map((q) => (
                  <QuantRow
                    key={q.artifactName}
                    quant={q}
                    started={started.has(q.artifactName)}
                    onDownload={() => baixar(q)}
                    ctxLen={view?.ctxLen ?? 8192}
                  />
                ))}
              </div>
            )}
          </div>

          {view && (
            <div className="mt-3 flex flex-col gap-1">
              {/* O projetor não é uma quantização: é acessório, e custa
                  memória à parte. */}
              {view.visionProjectorBytes != null && (
                <p className="text-[11px] leading-relaxed text-dim">
                  {t("badge.hasProjector", {
                    size: formatBytes(view.visionProjectorBytes),
                  })}
                </p>
              )}
              {view.modelCtxMax != null && (
                <p className="text-[11px] leading-relaxed text-dim">
                  {t("badge.modelCtxMax", {
                    n: (view.modelCtxMax / 1024).toFixed(0),
                  })}
                </p>
              )}
              <p className="text-[11px] leading-relaxed text-dim">
                {view.calibrated
                  ? t("badge.estimateCalibrated")
                  : t("badge.estimateOnly")}
              </p>
            </div>
          )}
        </section>

        {/* -------------------------------------------------- detalhes */}
        <section className="mt-6">
          <h3 className="text-sm font-medium text-ink">
            {t("discover.details")}
          </h3>
          <div className="mt-3 grid grid-cols-1 gap-2.5 rounded-xl border border-edge bg-panel p-4 sm:grid-cols-2">
            {model.paramsTotal != null && (
              <Campo label={t("discover.params")}>
                {formatParams(model.paramsTotal)}
              </Campo>
            )}
            {model.architecture && (
              <Campo label={t("discover.architecture")}>
                {model.architecture}
              </Campo>
            )}
            {model.contextLength != null && (
              <Campo label={t("discover.context")}>
                {formatCount(model.contextLength)} tokens
              </Campo>
            )}
            <Campo label={t("discover.format")}>GGUF</Campo>
            <div className="flex items-center gap-2 sm:col-span-2">
              <span className="shrink-0 text-[11px] text-dim">
                {t("discover.capabilities")}
              </span>
              <span className="flex flex-wrap items-center gap-1.5">
                {model.caps.vision || model.caps.tools || model.caps.reasoning ? (
                  <CapBadges caps={model.caps} withLabel />
                ) : (
                  <span className="text-[12px] text-dim">
                    {t("discover.caps.none")}
                  </span>
                )}
              </span>
            </div>
          </div>
        </section>

        {/* ---------------------------------------------------- README */}
        {readme !== "" && (
          <section className="mt-6">
            <h3 className="text-sm font-medium text-ink">
              {t("discover.readme")}
            </h3>
            <div className="mt-3 rounded-xl border border-edge bg-panel px-4 py-3">
              {readme == null ? (
                <div className="flex animate-pulse flex-col gap-2 py-2">
                  <div className="h-3 w-2/3 rounded bg-panel2" />
                  <div className="h-3 w-full rounded bg-panel2" />
                  <div className="h-3 w-4/5 rounded bg-panel2" />
                </div>
              ) : (
                <Markdown text={readme} />
              )}
            </div>
          </section>
        )}
      </div>
    </div>
  );
}
