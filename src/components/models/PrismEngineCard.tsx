// O cartão "este modelo precisa do motor da PrismML": o que é, quanto pesa e
// um botão que instala. Aparece no chat (no lugar da bolha de erro), na
// biblioteca (num modelo Bonsai sem o motor) e some sozinho quando o motor
// chega. O progresso vem do `prismStore`, então começar a instalação num
// lugar e olhar em outro mostra a mesma barra.
//
// O estado é o do ARQUIVO: no Linux com NVIDIA, um PQ2_0 pede a versão CUDA
// mesmo com o Vulkan instalado (no Vulkan ele roda na CPU). Onde a CUDA não
// é possível, o cartão vira um aviso que não bloqueia: o PQ2_0 roda, devagar,
// e o PTQ1_0 do mesmo repositório roda na GPU.

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { formatBytes } from "../../lib/format";
import {
  cancelarPrism,
  carregarPrism,
  ehCartaoDoChatAtivo,
  instalarPrism,
  prismCardMode,
  prismStore,
  registrarCartaoDoChat,
  tentarCudaDeNovo,
} from "../../lib/prism";

export default function PrismEngineCard({
  onInstalled,
  compact = false,
  model,
}: {
  /** Chamado quando o motor chega para o arquivo — o chat reenvia a mensagem. */
  onInstalled?: () => void;
  compact?: boolean;
  /**
   * O modelo em questão. Sem ele (o chat), vale o último que ouviu
   * `prism-required`; sem nenhum, o estado da máquina.
   */
  model?: string;
}) {
  const { t } = useTranslation();
  const snap = useSyncExternalStore(prismStore.subscribe, prismStore.get);
  const alvo = model ?? snap.modeloExigido ?? undefined;
  // A instalação terminou, mas numa reserva que roda o PQ2_0 na CPU (a
  // versão CUDA reprovou aqui): em vez de reenviar calado, o chat mostra o
  // aviso e deixa a pessoa decidir.
  const [aguardandoDecisao, setAguardandoDecisao] = useState(false);

  useEffect(() => {
    void carregarPrism(alvo);
  }, [alvo]);

  const status = alvo ? (snap.porModelo[alvo] ?? null) : snap.state;
  const modo = prismCardMode(status);

  // Só o cartão do chat montado por último avisa quando o motor chega.
  const [id] = useState(() => Symbol("cartao-prism"));
  const doChat = !!onInstalled;
  useEffect(() => (doChat ? registrarCartaoDoChat(id) : undefined), [doChat, id]);

  // O motor chegou PARA ESTE ARQUIVO — pelo botão daqui, por outro cartão ou
  // pela instalação que o download de um Bonsai disparou. É a passagem de
  // "não instalado" para "instalado" que destrava o chat, não o fim de uma
  // chamada: a instalação em segundo plano também reenvia a mensagem.
  const anterior = useRef<{ alvo: string | undefined; instalado: boolean | null }>({
    alvo,
    instalado: null,
  });
  const instalado = status?.installed ?? null;
  const naCpu = !!status?.pq2OnCpu;
  useEffect(() => {
    const antes = anterior.current;
    anterior.current = { alvo, instalado };
    const chegou = antes.alvo === alvo && antes.instalado === false && instalado === true;
    if (!chegou || !onInstalled || !ehCartaoDoChatAtivo(id)) return;
    if (naCpu) setAguardandoDecisao(true);
    else onInstalled();
  }, [alvo, instalado, naCpu, onInstalled, id]);

  const instalar = () => {
    void instalarPrism().catch(() => {
      /* o erro já está no store */
    });
  };
  const tentarCuda = () => {
    void tentarCudaDeNovo().catch(() => {
      /* o erro já está no store */
    });
  };

  const pct =
    snap.progress?.kind === "progress" && snap.progress.totalBytes > 0
      ? Math.min(100, (snap.progress.receivedBytes / snap.progress.totalBytes) * 100)
      : null;

  const progresso = (
    <div className="flex min-w-0 flex-1 items-center gap-2 text-[12px] text-dim">
      <span className="h-1.5 w-32 shrink-0 overflow-hidden rounded-full bg-panel2">
        <span
          className={`block h-full rounded-full bg-accent transition-[width] duration-300 ${pct == null ? "w-1/3 animate-pulse" : ""}`}
          style={pct == null ? undefined : { width: `${pct}%` }}
        />
      </span>
      <span className="truncate">
        {snap.progress?.kind === "extracting"
          ? t("settings.engine.extracting", { asset: snap.progress.asset })
          : t("models.prismInstalling")}
        {pct != null ? ` · ${pct.toFixed(0)}%` : ""}
      </span>
      <button
        type="button"
        onClick={() => void cancelarPrism()}
        className="shrink-0 rounded-md px-2 py-0.5 text-[12px] text-dim transition-colors hover:bg-panel2 hover:text-ink"
      >
        {t("models.prismCancel")}
      </button>
    </div>
  );

  // Nada a dizer sobre este arquivo: some — mesmo com uma instalação correndo
  // por outro modelo (o PTQ1_0 que já roda no Vulkan não precisa de nada).
  if (modo.kind === "hidden") {
    // No chat, a bolha não pode ficar vazia: a conversa reaberta depois de
    // reiniciar o app não sabe mais o modelo (e o motor pode ter chegado
    // desde então). Reenviar resolve os dois casos.
    if (!onInstalled || !status) return null;
    return (
      <div
        className={`rounded-xl border border-edge bg-panel ${compact ? "px-3 py-2" : "px-4 py-3"}`}
        role="status"
      >
        <p className="text-sm text-ink">{t("models.prismChatRetry")}</p>
        <div className="mt-2">
          <button
            type="button"
            onClick={onInstalled}
            className="rounded-lg border border-edge px-3 py-1.5 text-xs font-medium text-ink transition-colors hover:bg-panel2"
          >
            {t("models.prismRetrySend")}
          </button>
        </div>
      </div>
    );
  }

  if (modo.kind === "pq2OnCpu") {
    return (
      <div
        className={`rounded-xl border border-warn/40 bg-warn/5 ${compact ? "px-3 py-2" : "px-4 py-3"}`}
        role="status"
      >
        <p className="text-sm text-ink">{t("models.prismPq2OnCpu")}</p>
        {modo.cudaFailed && (
          <p className="mt-1 text-[12px] text-dim">{t("models.prismCudaFailed")}</p>
        )}
        <p className="mt-1 text-[12px] text-dim">{t("models.prismPtq1Hint")}</p>
        {snap.installing ? (
          <div className="mt-2 flex">{progresso}</div>
        ) : (
          (modo.cudaFailed || (aguardandoDecisao && onInstalled)) && (
            <div className="mt-2 flex flex-wrap items-center gap-2">
              {modo.cudaFailed && (
                <button
                  type="button"
                  onClick={tentarCuda}
                  className="rounded-lg border border-edge px-3 py-1.5 text-xs font-medium text-ink transition-colors hover:bg-panel2"
                >
                  {t("models.prismRetryCuda")}
                </button>
              )}
              {aguardandoDecisao && onInstalled && (
                <button
                  type="button"
                  onClick={() => {
                    setAguardandoDecisao(false);
                    onInstalled();
                  }}
                  className="rounded-lg border border-edge px-3 py-1.5 text-xs font-medium text-ink transition-colors hover:bg-panel2"
                >
                  {t("models.prismContinue")}
                </button>
              )}
            </div>
          )
        )}
        {snap.error && <p className="mt-2 text-[12px] text-bad">{snap.error}</p>}
      </div>
    );
  }

  const upgrade = modo.upgradeFrom;
  return (
    <div
      className={`rounded-xl border border-warn/40 bg-warn/5 ${compact ? "px-3 py-2" : "px-4 py-3"}`}
      role="status"
    >
      <p className="text-sm text-ink">
        {upgrade === "vulkan"
          ? t("models.prismVulkanPq2")
          : upgrade != null
            ? t("models.prismCpuPq2")
            : t("models.prismMissing")}
      </p>
      {!compact && <p className="mt-1 text-[12px] text-dim">{t("models.prismWhat")}</p>}
      {!compact && modo.diskBytes != null && (
        <p className="mt-1 text-[12px] text-dim">
          {t("models.prismDisk", { size: formatBytes(modo.diskBytes) })}
        </p>
      )}
      <div className="mt-2 flex flex-wrap items-center gap-3">
        {snap.installing ? (
          progresso
        ) : (
          <button
            type="button"
            onClick={instalar}
            className="rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-white transition-opacity hover:opacity-90"
          >
            {upgrade != null
              ? t("models.prismInstallCuda", { size: formatBytes(modo.bytes) })
              : t("models.prismInstall", { size: formatBytes(modo.bytes) })}
          </button>
        )}
      </div>
      {snap.error && <p className="mt-2 text-[12px] text-bad">{snap.error}</p>}
    </div>
  );
}
