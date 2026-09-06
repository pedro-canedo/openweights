// DeepSeek Harness — tela própria, no mesmo nível do Chat.
//
// O agente de código não é um botão escondido num canto do Chat: é um dos
// modos do aplicativo, e por isso tem item exclusivo na barra lateral. Aqui a
// pessoa vê e comanda o ciclo de vida inteiro — instalar, subir, atualizar,
// parar, desinstalar — e usa o harness DENTRO do app, num quadro embutido.
// Nada disso exige terminal, npm global ou caçar pasta no disco: o app
// instala numa pasta sua, sobe em porta efêmera de loopback e derruba junto.
//
// A janela própria continua existindo como saída de emergência (outro
// monitor, ou um dia em que o servidor recuse ser embutido), mas deixou de
// ser o caminho padrão.
//
// Sobre a versão: a tela dizia "Instalado — versão 0.1.1-rc.2" lendo a
// versão que o app *instalaria*, não a que estava no disco. Quem atualizasse
// o OpenWeights leria a versão nova e continuaria rodando a antiga. Agora a
// versão vem do `package.json` do pacote instalado, a comparação com a que
// esta build traz acontece sozinha ao abrir a tela, e a divergência aparece
// como pendência — com o botão que a resolve ao lado.

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
  verificarAtualizacao,
} from "../lib/harness";
import { formatBytes, formatEta } from "../lib/format";
import { Card, Page, StatusDot } from "../components/ui/Shell";

const botao =
  "rounded-lg border border-edge px-3 py-2 text-sm text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-50";
const botaoPrimario =
  "rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white disabled:opacity-50";

export default function Harness() {
  const s = useSyncExternalStore(harnessStore.subscribe, harnessStore.get);

  // Force: o processo pode ter morrido desde a última leitura (ou o app foi
  // reaberto), e esta é a tela em que a verdade precisa estar em dia. A
  // verificação de versão vem junto: é a pergunta que a pessoa faria se
  // soubesse que pode fazê-la.
  useEffect(() => {
    void refreshStatus(true);
    void verificarAtualizacao();
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
        {/* A pendência acompanha quem está usando: parar para atualizar é
            decisão dela, mas a informação não pode ficar só na outra tela. */}
        {s.check?.updatePending && (
          <span className="rounded-full border border-warn/40 bg-warn/10 px-2 py-0.5 text-[10px] text-warn">
            {t("harness.update.pendingShort", {
              version: s.check.pinnedVersion,
            })}
          </span>
        )}
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

/** Fora do ar: instalar, subir, atualizar e remover — com progresso à vista. */
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
  const pendente = s.check?.updatePending ?? false;
  const versao = s.status?.version || s.check?.version || "";

  return (
    <div className="h-full overflow-y-auto">
      <Page title={t("harness.title")} subtitle={t("harness.subtitle")}>
        {/* ------------------------------------------------- estado + ação */}
        <Card tone={pendente ? "warn" : "normal"}>
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <StatusDot
                  tone={pendente ? "warn" : instalado ? "ok" : "off"}
                  pulse={s.busy !== null}
                />
                <span className="text-sm font-medium">
                  {instalado
                    ? t("harness.stateInstalled", { version: versao })
                    : t("harness.stateNotInstalled")}
                </span>
                {pendente && (
                  <span className="rounded-full border border-warn/40 bg-warn/10 px-2 py-0.5 text-[10px] text-warn">
                    {t("harness.update.pendingBadge")}
                  </span>
                )}
              </div>
              <p className="mt-1 max-w-xl text-[12px] leading-relaxed text-dim">
                {pendente
                  ? t("harness.update.pendingBody", {
                      installed: s.check?.version,
                      pinned: s.check?.pinnedVersion,
                    })
                  : instalado
                    ? t("harness.readyHint")
                    : t("harness.sizeWarning")}
              </p>
            </div>

            <div className="flex shrink-0 flex-wrap gap-2">
              {pendente && (
                <button
                  onClick={() => void instalar()}
                  disabled={s.busy !== null}
                  className={botaoPrimario}
                >
                  {s.busy === "install"
                    ? t("common.loading")
                    : t("harness.update.apply")}
                </button>
              )}
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
                className={pendente ? botao : botaoPrimario}
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
                  className="brand-gradient h-full rounded-full transition-[width]"
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
                <div className="brand-gradient h-full w-1/3 animate-pulse rounded-full" />
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
        </Card>

        {/* --------------------------------------------------- atualização */}
        <Card
          title={t("harness.update.title")}
          hint={t("harness.update.hint")}
          action={
            <button
              onClick={() => void verificarAtualizacao()}
              disabled={s.checking}
              className="rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-50"
            >
              {s.checking
                ? t("harness.update.checking")
                : t("harness.update.check")}
            </button>
          }
        >
          <dl className="mt-3 grid grid-cols-2 gap-x-8 gap-y-1.5 text-[12px]">
            <div>
              <dt className="text-dim">{t("harness.update.onDisk")}</dt>
              <dd>{versao || t("harness.notInstalled")}</dd>
            </div>
            <div>
              <dt className="text-dim">{t("harness.update.shipped")}</dt>
              <dd>{s.check?.pinnedVersion ?? s.status?.pinnedVersion ?? "—"}</dd>
            </div>
            <div>
              <dt className="text-dim">{t("harness.update.npm")}</dt>
              <dd>{s.check?.latestNpm ?? "—"}</dd>
            </div>
          </dl>
          {s.check?.npmNewer && s.check.latestNpm && (
            <p className="mt-3 text-[11px] leading-relaxed text-dim">
              {t("harness.update.npmNote", { version: s.check.latestNpm })}
            </p>
          )}
        </Card>

        {/* ---------------------------------------------------- isolamento */}
        <Card title={t("harness.controlTitle")} hint={t("harness.controlBody")}>
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
        </Card>
      </Page>
    </div>
  );
}
