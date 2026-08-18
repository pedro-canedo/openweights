// Acesso ao painel do 9router.
//
// Já foi um `<iframe>` embutido, e não podia dar certo: o 9router grava a
// sessão num cookie `auth_token` com `SameSite=Lax` e sem `Secure`. Dentro
// de um iframe o painel (`http://127.0.0.1:<porta>`) é cross-site em relação
// à origem do webview do app, e o Chromium descarta o cookie — o login
// respondia sucesso e a tela voltava ao formulário, com toda a cara de senha
// errada. Numa janela de primeiro nível o cookie é first-party e o painel se
// comporta como no navegador.
//
// Por isso são dois caminhos, ambos de primeiro nível: a janela do próprio
// app (padrão, não tira a pessoa daqui) e o navegador do sistema.

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { nineRouterOpenPanel } from "../../lib/providers";
import { openUrl } from "../../lib/openExternal";

export default function NineRouterPanel({ url }: { url: string }) {
  const { t } = useTranslation();
  const [erro, setErro] = useState<string | null>(null);

  return (
    <div className="rounded-xl border border-edge bg-panel p-4">
      <div className="flex flex-wrap items-center gap-3">
        <button
          onClick={() => {
            setErro(null);
            void nineRouterOpenPanel().catch((e) => setErro(String(e)));
          }}
          className="rounded-lg bg-accent px-3 py-2 text-sm font-medium text-white"
        >
          {t("providers.nineRouter.openPanel")}
        </button>
        <button
          onClick={() => void openUrl(url)}
          className="rounded-lg border border-edge px-3 py-2 text-sm text-dim hover:text-ink"
        >
          {t("providers.nineRouter.openExternal")}
        </button>
        <span className="truncate text-[11px] text-dim">{url}</span>
      </div>

      <p className="mt-2 text-[11px] text-dim">
        {t("providers.nineRouter.panelHint")}
      </p>

      {erro && (
        <p className="mt-2 rounded-lg border border-bad/40 bg-bad/10 px-3 py-2 text-[12px] text-bad">
          {erro}
        </p>
      )}
    </div>
  );
}
