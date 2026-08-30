// A experiência "agente" do app agora é o DeepSeek Harness: um agente de
// código completo, aberto em janela própria e pré-configurado pelo backend
// (`dsh_start`) com todos os provedores e modelos que o app conhece.
//
// O estado do lançamento mora num store de módulo porque o CTA aparece em
// dois lugares ao mesmo tempo (card no hero + botão compacto no composer) e
// os dois precisam contar a mesma história. O overlay de progresso é montado
// UMA vez pela tela de Chat — flutuante, para não bloquear a conversa
// enquanto a primeira instalação (Node + npm install) acontece.

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import {
  dshOpenPanel,
  dshStart,
  dshStatus,
  onProviderEvent,
  type DshStatus,
  type ProviderEvent,
} from "../../lib/providers";
import { errorMessage } from "../../lib/serverSession";
import { formatBytes, formatEta } from "../../lib/format";

interface LaunchState {
  status: DshStatus | null;
  busy: boolean;
  progress: ProviderEvent | null;
  log: string[];
  error: string | null;
  /** true quando o usuário fechou o overlay (a instalação continua). */
  dismissed: boolean;
}

let state: LaunchState = {
  status: null,
  busy: false,
  progress: null,
  log: [],
  error: null,
  dismissed: false,
};

const listeners = new Set<() => void>();

function set(patch: Partial<LaunchState>) {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

const store = {
  subscribe(l: () => void) {
    listeners.add(l);
    return () => {
      listeners.delete(l);
    };
  },
  get: () => state,
};

// Uma busca de status por sessão basta: quem muda o estado depois é o próprio
// launch(). Falha silenciosa deixa o CTA no caminho padrão (instalar+abrir).
let statusFetched = false;
async function refreshStatus(): Promise<void> {
  if (statusFetched) return;
  statusFetched = true;
  try {
    set({ status: await dshStatus() });
  } catch {
    // sem backend (navegador) ou comando indisponível
  }
}

async function launch(): Promise<void> {
  if (state.busy) return;
  set({
    busy: true,
    error: null,
    log: [],
    progress: null,
    dismissed: false,
  });
  // Mesma guarda de StrictMode do NineRouterCard: o unlisten pode chegar
  // depois de um segundo registro e deixaria assinatura pendurada.
  const sub: { cancelado: boolean; desligar: (() => void) | null } = {
    cancelado: false,
    desligar: null,
  };
  void onProviderEvent((e) => {
    set({
      progress: e,
      log: e.kind === "log" ? [...state.log.slice(-200), e.line] : state.log,
    });
  }).then((f) => {
    if (sub.cancelado) f();
    else sub.desligar = f;
  });
  try {
    await dshStart();
    // A verdade vem do status, não do retorno do start (contrato mínimo).
    const s = await dshStatus().catch(() => null);
    if (s) set({ status: s });
    await dshOpenPanel();
  } catch (e) {
    set({ error: errorMessage(e) });
  } finally {
    sub.cancelado = true;
    sub.desligar?.();
    set({ busy: false, progress: null });
  }
}

/** Um clique: rodando → só abre o painel; senão instala/sobe e abre. */
async function open(): Promise<void> {
  if (state.busy) return;
  if (state.status?.running) {
    try {
      await dshOpenPanel();
      return;
    } catch {
      // o processo pode ter caído desde o último status — caminho completo
    }
  }
  await launch();
}

/** Card no estado vazio do Chat (hero): o convite explícito. */
export function HarnessHeroCard() {
  const { t } = useTranslation();
  const s = useSyncExternalStore(store.subscribe, store.get);
  useEffect(() => {
    void refreshStatus();
  }, []);
  const running = s.status?.running ?? false;

  return (
    <div className="mt-4 flex w-full items-center gap-4 rounded-2xl border border-edge bg-panel/80 px-5 py-4 text-left">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-ink">
          {t("chat.harness.title")}
        </div>
        <p className="mt-1 text-[12px] leading-relaxed text-dim">
          {t("chat.harness.body")}
        </p>
      </div>
      <button
        type="button"
        onClick={() => void open()}
        disabled={s.busy}
        className="shrink-0 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white disabled:opacity-50"
      >
        {s.busy
          ? t("common.loading")
          : running
            ? t("chat.harness.openPanel")
            : t("chat.harness.open")}
      </button>
    </div>
  );
}

/** Botão compacto do composer — mora onde vivia o toggle de agente. */
export function HarnessComposerButton() {
  const { t } = useTranslation();
  const s = useSyncExternalStore(store.subscribe, store.get);
  useEffect(() => {
    void refreshStatus();
  }, []);
  const running = s.status?.running ?? false;

  return (
    <button
      type="button"
      onClick={() => void open()}
      disabled={s.busy}
      title={running ? t("chat.harness.openPanel") : t("chat.harness.title")}
      className={`flex h-8 shrink-0 items-center gap-1 rounded-full border border-edge px-3 text-xs transition-colors disabled:opacity-50 ${
        s.busy ? "animate-pulse text-dim" : "text-dim hover:border-accent hover:text-ink"
      }`}
    >
      <span>{t("chat.harness.agent")}</span>
      <span aria-hidden="true">⧉</span>
    </button>
  );
}

/**
 * Progresso/erro do lançamento, flutuante — montado uma única vez pela tela
 * de Chat. Padrão do NineRouterCard: barra real quando o download tem tamanho
 * conhecido, faixa indeterminada + fase + cronômetro para o npm, log ao vivo.
 */
export function HarnessLaunchOverlay() {
  const { t } = useTranslation();
  const s = useSyncExternalStore(store.subscribe, store.get);
  const logRef = useRef<HTMLPreElement>(null);
  const [segundos, setSegundos] = useState(0);

  useEffect(() => {
    if (!s.busy) {
      setSegundos(0);
      return;
    }
    const id = window.setInterval(() => setSegundos((x) => x + 1), 1000);
    return () => window.clearInterval(id);
  }, [s.busy]);

  useEffect(() => {
    logRef.current?.scrollTo(0, logRef.current.scrollHeight);
  }, [s.log]);

  if (s.dismissed || (!s.busy && !s.error)) return null;

  const p = s.progress;
  const instalando =
    s.log.length > 0 ||
    p?.kind === "progress" ||
    p?.kind === "extracting" ||
    p?.kind === "installing";

  return (
    <div className="fixed right-4 bottom-24 z-30 w-80 rounded-xl border border-edge bg-panel p-4 shadow-2xl">
      <div className="flex items-center justify-between gap-2">
        <div className="text-sm font-medium">DeepSeek Harness</div>
        <button
          type="button"
          onClick={() => set({ dismissed: true })}
          title={t("common.close")}
          className="rounded px-1 text-dim hover:text-ink"
        >
          ×
        </button>
      </div>

      {s.busy &&
        (p?.kind === "progress" && p.totalBytes > 0 ? (
          <div className="mt-3">
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-panel2">
              <div
                className="h-full rounded-full bg-accent transition-[width]"
                style={{
                  width: `${(p.receivedBytes / p.totalBytes) * 100}%`,
                }}
              />
            </div>
            <div className="mt-1 text-[11px] text-dim">
              {formatBytes(p.receivedBytes)} / {formatBytes(p.totalBytes)} —{" "}
              {p.asset}
            </div>
          </div>
        ) : (
          <div className="mt-3">
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-panel2">
              <div className="h-full w-1/3 animate-pulse rounded-full bg-accent" />
            </div>
            <div className="mt-1 text-[11px] text-dim">
              {p?.kind === "installing"
                ? t(`chat.harness.phase.${p.phase}`, {
                    defaultValue: p.phase,
                  })
                : p?.kind === "extracting"
                  ? t("chat.harness.phase.extracting")
                  : t("chat.harness.starting")}{" "}
              · {formatEta(segundos)}
            </div>
          </div>
        ))}

      {s.busy && instalando && (
        <p className="mt-2 text-[11px] text-dim">
          {t("chat.harness.installHint")}
        </p>
      )}

      {s.busy && s.log.length > 0 && (
        <pre
          ref={logRef}
          className="mt-2 max-h-32 overflow-y-auto rounded-lg border border-edge bg-panel2 p-2 text-[10px] leading-relaxed text-dim"
        >
          {s.log.join("\n")}
        </pre>
      )}

      {s.error && (
        <p className="mt-2 rounded-lg border border-bad/40 bg-bad/10 px-3 py-2 text-[12px] text-bad">
          {t("chat.harness.error")}: {s.error}
        </p>
      )}
    </div>
  );
}
