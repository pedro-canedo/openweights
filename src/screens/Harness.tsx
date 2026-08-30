// DeepSeek Harness — tela própria, no mesmo nível do Chat.
//
// O agente de código não é um botão escondido num canto do Chat: é um dos
// modos do aplicativo, e por isso tem item exclusivo na barra lateral. Aqui a
// pessoa vê e comanda o ciclo de vida inteiro — instalar, subir, parar,
// desinstalar — e usa o harness DENTRO do app, num quadro embutido. Nada
// disso exige terminal, npm global ou caçar pasta no disco: o app instala
// numa pasta sua, sobe em porta efêmera de loopback e derruba junto.
//
// A janela própria continua existindo como saída de emergência (outro
// monitor, ou um dia em que o servidor recuse ser embutido), mas deixou de
// ser o caminho padrão.

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import {
  abrirJanela,
  desinstalar,
  harnessStore,
  instalar,
  parar,
  refreshStatus,
  subir,
} from "../lib/harness";
import { formatBytes, formatEta } from "../lib/format";

const botao =
  "rounded-lg border border-edge px-3 py-2 text-sm text-dim transition-colors hover:text-ink disabled:opacity-50";
const botaoPrimario =
  "rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white disabled:opacity-50";

export default function Harness() {
  const s = useSyncExternalStore(harnessStore.subscribe, harnessStore.get);

  // Force: o processo pode ter morrido desde a última leitura (ou o app foi
  // reaberto), e esta é a tela em que a verdade precisa estar em dia.
  useEffect(() => {
    void refreshStatus(true);
  }, []);

  const url = s.status?.panelUrl ?? null;
  return s.status?.running && url ? <Palco url={url} /> : <Controle />;
}

/** No ar: o harness ocupa a tela, com uma barra fina de comando em cima. */
function Palco({ url }: { url: string }) {
  const { t } = useTranslation();
  const s = useSyncExternalStore(harnessStore.subscribe, harnessStore.get);
  // Recarregar é trocar a `key` do iframe: o conteúdo é de outra origem, não
  // há como falar com ele por dentro.
  const [recarga, setRecarga] = useState(0);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-edge bg-panel px-4 py-2">
        <span className="text-sm font-medium">{t("harness.title")}</span>
        <span className="rounded-full border border-ok/40 bg-ok/10 px-2 py-0.5 text-[10px] text-ok">
          {t("harness.running", { port: s.status?.port })}
        </span>
        <span className="truncate text-[11px] text-dim">{url}</span>
        <div className="ml-auto flex flex-wrap gap-2">
          <button onClick={() => setRecarga((n) => n + 1)} className={botao}>
            {t("harness.reload")}
          </button>
          <button onClick={() => void abrirJanela()} className={botao}>
            {t("harness.openWindow")}
          </button>
          <button
            onClick={() => void parar()}
            disabled={s.busy !== null}
            className={botao}
          >
            {s.busy === "stop" ? t("common.loading") : t("harness.stop")}
          </button>
        </div>
      </div>

      <iframe
        key={recarga}
        src={url}
        title={t("harness.title")}
        className="min-h-0 flex-1 border-0 bg-panel2"
        allow="clipboard-read; clipboard-write; microphone"
      />

      <p className="shrink-0 border-t border-edge px-4 py-1.5 text-[11px] text-dim">
        {t("harness.embedHint")}
      </p>
    </div>
  );
}

/** Fora do ar: instalar, subir e desinstalar, com progresso e log à vista. */
function Controle() {
  const { t } = useTranslation();
  const s = useSyncExternalStore(harnessStore.subscribe, harnessStore.get);
  const [confirmando, setConfirmando] = useState(false);
  const logRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    logRef.current?.scrollTo(0, logRef.current.scrollHeight);
  }, [s.log]);

  const instalado = s.status?.installed ?? false;
  const baixando = s.progress?.kind === "progress" && s.progress.totalBytes > 0;

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-3xl px-6 py-6">
        <h1 className="text-lg font-medium">{t("harness.title")}</h1>
        <p className="mt-1 text-[13px] leading-relaxed text-dim">
          {t("harness.subtitle")}
        </p>

        <div className="mt-5 rounded-xl border border-edge bg-panel px-5 py-4">
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-sm">
                  {instalado
                    ? t("harness.stateInstalled", { version: s.status?.version })
                    : t("harness.stateNotInstalled")}
                </span>
                <span
                  className={`rounded-full border px-2 py-0.5 text-[10px] ${
                    instalado
                      ? "border-ok/40 bg-ok/10 text-ok"
                      : "border-edge text-dim"
                  }`}
                >
                  {instalado
                    ? t("harness.installed")
                    : t("harness.notInstalled")}
                </span>
              </div>
              <p className="mt-1 text-[12px] leading-relaxed text-dim">
                {instalado ? t("harness.readyHint") : t("harness.sizeWarning")}
              </p>
            </div>

            <div className="flex shrink-0 flex-wrap gap-2">
              {!instalado && (
                <button
                  onClick={() => void instalar()}
                  disabled={s.busy !== null}
                  className={botao}
                >
                  {s.busy === "install"
                    ? t("common.loading")
                    : t("harness.install")}
                </button>
              )}
              <button
                onClick={() => void subir()}
                disabled={s.busy !== null}
                className={botaoPrimario}
              >
                {s.busy === "start"
                  ? t("common.loading")
                  : instalado
                    ? t("harness.start")
                    : t("harness.installAndStart")}
              </button>
            </div>
          </div>

          {/* O download do Node tem tamanho conhecido e ganha barra de
              verdade; o npm não publica porcentagem — ali vão fase, log ao
              vivo e cronômetro. Uma barra que avançasse sozinha mentiria. */}
          {baixando && s.progress?.kind === "progress" && (
            <div className="mt-4">
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-panel2">
                <div
                  className="h-full rounded-full bg-accent transition-[width]"
                  style={{
                    width: `${(s.progress.receivedBytes / s.progress.totalBytes) * 100}%`,
                  }}
                />
              </div>
              <div className="mt-1 text-[11px] text-dim">
                {formatBytes(s.progress.receivedBytes)} /{" "}
                {formatBytes(s.progress.totalBytes)} — {s.progress.asset}
              </div>
            </div>
          )}

          {s.busy && !baixando && (
            <div className="mt-4">
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-panel2">
                <div className="h-full w-1/3 animate-pulse rounded-full bg-accent" />
              </div>
              <div className="mt-1 text-[11px] text-dim">
                {s.progress?.kind === "installing"
                  ? t(`harness.phase.${s.progress.phase}`, {
                      defaultValue: s.progress.phase,
                    })
                  : s.progress?.kind === "extracting"
                    ? t("harness.phase.extracting")
                    : t(`harness.working.${s.busy}`)}{" "}
                · {formatEta(s.segundos)}
              </div>
            </div>
          )}

          {(s.busy === "install" || s.busy === "start") && (
            <p className="mt-2 text-[11px] leading-relaxed text-dim">
              {t("harness.installHint")}
            </p>
          )}

          {s.log.length > 0 && (
            <pre
              ref={logRef}
              className="mt-3 max-h-48 overflow-y-auto rounded-lg border border-edge bg-panel2 p-2 text-[10px] leading-relaxed text-dim"
            >
              {s.log.join("\n")}
            </pre>
          )}

          {s.error && (
            <p className="mt-3 rounded-lg border border-bad/40 bg-bad/10 px-3 py-2 text-[12px] text-bad">
              {t("harness.error")}: {s.error}
            </p>
          )}
        </div>

        <div className="mt-5 rounded-xl border border-edge bg-panel px-5 py-4">
          <div className="text-sm">{t("harness.controlTitle")}</div>
          <p className="mt-1 text-[12px] leading-relaxed text-dim">
            {t("harness.controlBody")}
          </p>
          <ul className="mt-3 space-y-1.5 text-[12px] text-dim">
            <li>· {t("harness.controlIsolated")}</li>
            <li>· {t("harness.controlProviders")}</li>
            <li>· {t("harness.controlKeys")}</li>
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
                  {t("harness.uninstall")}
                </button>
              ) : (
                <div className="rounded-lg border border-bad/40 bg-bad/10 px-3 py-2">
                  <p className="text-[12px] text-bad">
                    {t("harness.uninstallWarning")}
                  </p>
                  <div className="mt-2 flex flex-wrap gap-2">
                    <button
                      onClick={() => {
                        setConfirmando(false);
                        void desinstalar(true);
                      }}
                      className="rounded-lg bg-bad px-3 py-1.5 text-[12px] font-medium text-white"
                    >
                      {t("harness.uninstallAll")}
                    </button>
                    <button
                      onClick={() => {
                        setConfirmando(false);
                        void desinstalar(false);
                      }}
                      className={botao}
                    >
                      {t("harness.uninstallKeepData")}
                    </button>
                    <button
                      onClick={() => setConfirmando(false)}
                      className={botao}
                    >
                      {t("common.cancel")}
                    </button>
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
