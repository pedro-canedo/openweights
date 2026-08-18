// O painel do 9router embutido no app.
//
// Funciona porque o 9router não define `X-Frame-Options` nem
// `frame-ancestors` (confirmado no `next.config` do pacote) — ao contrário do
// site do OpenRouter, que bloqueia os dois e por isso ganhou tela nativa.
//
// `allow-same-origin` é necessário, não conveniência: sem ele o documento
// roda em origem opaca, o cookie de sessão não grava e o login simplesmente
// não acontece. O risco é contido porque a origem do iframe
// (`http://127.0.0.1:<porta>`) NÃO é a do app (`tauri://localhost`): é
// cross-origin, sem acesso ao DOM do app, e a capability do Tauri está presa
// à janela `main` sem declarar `remote`, então o iframe não recebe IPC.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "../../lib/openExternal";

export default function NineRouterFrame({ url }: { url: string }) {
  const { t } = useTranslation();
  const [carregou, setCarregou] = useState(false);
  const [demorou, setDemorou] = useState(false);
  const quadro = useRef<HTMLIFrameElement>(null);

  // Frame bloqueado não dispara `onerror` de forma confiável — em vários
  // navegadores não dispara nada. O relógio é o único jeito de perceber, e
  // por isso o botão de abrir fora existe sempre, não só no erro.
  useEffect(() => {
    setCarregou(false);
    setDemorou(false);
    const id = window.setTimeout(() => setDemorou(true), 5000);
    return () => window.clearTimeout(id);
  }, [url]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center justify-between gap-3 px-1 pb-2">
        <span className="truncate text-[11px] text-dim">{url}</span>
        <button
          onClick={() => void openUrl(url)}
          className="shrink-0 text-[11px] text-dim underline hover:text-ink"
        >
          {t("providers.nineRouter.openExternal")}
        </button>
      </div>

      {!carregou && demorou && (
        <p className="mb-2 rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-[12px] text-warn">
          {t("providers.nineRouter.frameBlocked")}
        </p>
      )}

      <iframe
        ref={quadro}
        src={url}
        title={t("providers.nineRouter.title")}
        onLoad={() => setCarregou(true)}
        sandbox="allow-scripts allow-same-origin allow-forms allow-downloads"
        className="min-h-[32rem] w-full flex-1 rounded-xl border border-edge bg-white"
      />
    </div>
  );
}
