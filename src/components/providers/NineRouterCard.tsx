// 9router: instalar, ligar, desligar e remover — tudo isolado numa pasta.
//
// O progresso tem duas naturezas e a tela não finge o contrário: o download
// do Node tem tamanho conhecido e ganha barra de verdade; o `npm install` não
// publica porcentagem, então ali aparece fase nomeada, log ao vivo e um
// cronômetro. Uma barra que avançasse sozinha mentiria sobre quanto falta.

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  nineRouterInstall,
  nineRouterStart,
  nineRouterStatus,
  nineRouterStop,
  nineRouterUninstall,
  onProviderEvent,
  type NineRouterStatus,
  type ProviderEvent,
} from "../../lib/providers";
import { errorMessage } from "../../lib/serverSession";
import { formatBytes } from "../../lib/format";

const botao =
  "rounded-lg border border-edge px-3 py-2 text-sm text-dim hover:text-ink disabled:opacity-50";

export default function NineRouterCard({
  onChanged,
}: {
  onChanged: (s: NineRouterStatus) => void;
}) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<NineRouterStatus | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [progresso, setProgresso] = useState<ProviderEvent | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const [segundos, setSegundos] = useState(0);
  const [confirmando, setConfirmando] = useState(false);
  const logRef = useRef<HTMLPreElement>(null);

  const recarregar = useCallback(async () => {
    try {
      const s = await nineRouterStatus();
      setStatus(s);
      onChanged(s);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [onChanged]);

  useEffect(() => {
    void recarregar();
  }, [recarregar]);

  // Cronômetro: com o npm mudo por minutos, é o que diz que ainda anda.
  useEffect(() => {
    if (!busy) {
      setSegundos(0);
      return;
    }
    const id = window.setInterval(() => setSegundos((s) => s + 1), 1000);
    return () => window.clearInterval(id);
  }, [busy]);

  async function comEventos(nome: string, acao: () => Promise<NineRouterStatus>) {
    setBusy(nome);
    setError(null);
    setLog([]);
    setProgresso(null);
    // StrictMode monta duas vezes: sem a bandeira, o primeiro `unlisten`
    // chegaria depois do segundo registro e sobraria assinatura pendurada.
    const assinatura: { cancelado: boolean; desligar: (() => void) | null } = {
      cancelado: false,
      desligar: null,
    };
    void onProviderEvent((e) => {
      setProgresso(e);
      if (e.kind === "log") {
        setLog((atual) => [...atual.slice(-200), e.line]);
      }
    }).then((f) => {
      if (assinatura.cancelado) f();
      else assinatura.desligar = f;
    });
    try {
      const s = await acao();
      setStatus(s);
      onChanged(s);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      assinatura.cancelado = true;
      assinatura.desligar?.();
      setBusy(null);
      setProgresso(null);
    }
  }

  useEffect(() => {
    logRef.current?.scrollTo(0, logRef.current.scrollHeight);
  }, [log]);

  const instalado = status?.installed ?? false;
  const rodando = status?.running ?? false;

  return (
    <div className="rounded-xl border border-edge bg-panel px-5 py-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="text-sm">{t("providers.nineRouter.title")}</div>
          <div className="mt-1 text-[12px] text-dim">
            {t("providers.nineRouter.subtitle")}
          </div>
        </div>
        <div className="flex shrink-0 gap-2">
          {!instalado && (
            <button
              onClick={() => void comEventos("install", nineRouterInstall)}
              disabled={busy !== null}
              className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white disabled:opacity-50"
            >
              {busy === "install"
                ? t("common.loading")
                : t("providers.nineRouter.install")}
            </button>
          )}
          {instalado && !rodando && (
            <button
              onClick={() => void comEventos("start", nineRouterStart)}
              disabled={busy !== null}
              className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white disabled:opacity-50"
            >
              {busy === "start" ? t("common.loading") : t("providers.nineRouter.start")}
            </button>
          )}
          {instalado && rodando && (
            <button
              onClick={() => void comEventos("stop", nineRouterStop)}
              disabled={busy !== null}
              className={botao}
            >
              {t("providers.nineRouter.stop")}
            </button>
          )}
        </div>
      </div>

      {!instalado && (
        <p className="mt-3 text-[11px] text-dim">
          {t("providers.nineRouter.sizeWarning")}
        </p>
      )}

      {/* Download com tamanho conhecido: barra de verdade. */}
      {progresso?.kind === "progress" && progresso.totalBytes > 0 && (
        <div className="mt-3">
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-panel2">
            <div
              className="h-full rounded-full bg-accent transition-[width]"
              style={{
                width: `${(progresso.receivedBytes / progresso.totalBytes) * 100}%`,
              }}
            />
          </div>
          <div className="mt-1 text-[11px] text-dim">
            {formatBytes(progresso.receivedBytes)} /{" "}
            {formatBytes(progresso.totalBytes)} — {progresso.asset}
          </div>
        </div>
      )}

      {/* npm: sem porcentagem honesta possível — faixa indeterminada. */}
      {busy && progresso?.kind !== "progress" && (
        <div className="mt-3">
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-panel2">
            <div className="h-full w-1/3 animate-pulse rounded-full bg-accent" />
          </div>
          <div className="mt-1 text-[11px] text-dim">
            {progresso?.kind === "installing"
              ? t(`providers.nineRouter.phase.${progresso.phase}`, {
                  defaultValue: progresso.phase,
                })
              : progresso?.kind === "extracting"
                ? t("providers.nineRouter.phase.extracting")
                : t("providers.nineRouter.working")}{" "}
            · {segundos}s
          </div>
        </div>
      )}

      {log.length > 0 && (
        <pre
          ref={logRef}
          className="mt-3 max-h-40 overflow-y-auto rounded-lg border border-edge bg-panel2 p-2 text-[11px] leading-relaxed text-dim"
        >
          {log.join("\n")}
        </pre>
      )}

      {error && (
        <p className="mt-2 rounded-lg border border-bad/40 bg-bad/10 px-3 py-2 text-[12px] text-bad">
          {error}
        </p>
      )}

      {instalado && (
        <div className="mt-4 border-t border-edge pt-3">
          <div className="flex flex-wrap items-center gap-3 text-[12px] text-dim">
            <span>
              {t("providers.nineRouter.version", { version: status?.version })}
            </span>
            <span>·</span>
            <span>{t("providers.nineRouter.port", { port: status?.port })}</span>
          </div>

          {status?.password && (
            <div className="mt-2 flex flex-wrap items-center gap-2 text-[12px]">
              <span className="text-dim">
                {t("providers.nineRouter.passwordLabel")}
              </span>
              <code className="rounded bg-panel2 px-2 py-1 font-mono">
                {status.password}
              </code>
              <button
                onClick={() => void navigator.clipboard.writeText(status.password)}
                className="text-dim underline hover:text-ink"
              >
                {t("common.copy")}
              </button>
            </div>
          )}
          <p className="mt-1 text-[11px] text-dim">
            {t("providers.nineRouter.passwordHint")}
          </p>

          {/* Remoção em dois passos, como o descarte de modelo. */}
          <div className="mt-3">
            {!confirmando ? (
              <button
                onClick={() => setConfirmando(true)}
                disabled={busy !== null}
                className="rounded-lg border border-edge px-3 py-2 text-sm text-dim hover:border-bad/60 hover:text-bad disabled:opacity-50"
              >
                {t("providers.nineRouter.uninstall")}
              </button>
            ) : (
              <div className="rounded-lg border border-bad/40 bg-bad/10 px-3 py-2">
                <p className="text-[12px] text-bad">
                  {t("providers.nineRouter.uninstallWarning")}
                </p>
                <div className="mt-2 flex flex-wrap gap-2">
                  <button
                    onClick={() => {
                      setConfirmando(false);
                      void comEventos("uninstall", () => nineRouterUninstall(true));
                    }}
                    className="rounded-lg bg-bad px-3 py-1.5 text-[12px] font-medium text-white"
                  >
                    {t("providers.nineRouter.uninstallAll")}
                  </button>
                  <button
                    onClick={() => {
                      setConfirmando(false);
                      void comEventos("uninstall", () => nineRouterUninstall(false));
                    }}
                    className={botao}
                  >
                    {t("providers.nineRouter.uninstallKeepData")}
                  </button>
                  <button onClick={() => setConfirmando(false)} className={botao}>
                    {t("common.cancel")}
                  </button>
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
