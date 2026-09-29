// "Copiar requisição": o que o chat mandaria agora, como curl, Python ou
// JavaScript. A chave de API nunca é copiada — vai como variável de ambiente.

import { useTranslation } from "react-i18next";
import { Button } from "../ui/Button";
import { toast } from "../ui/Toast";
import { getServerStatus, getSetting } from "../../lib/api";
import { buildChatBody, type ChatMessage } from "../../lib/llama";
import { providerEndpoint, splitModelRef } from "../../lib/providers";
import { gerarRequisicao, type FormatoDaRequisicao } from "../../lib/requisicao";
import type { ChatParams } from "../../lib/types";

const PADRAO_LOCAL = "http://127.0.0.1:11711";

/** Para onde a requisição iria e se essa fonte pede chave. */
async function destino(modelRef: string): Promise<{ baseUrl: string; usaChave: boolean }> {
  const { provider } = splitModelRef(modelRef);
  if (provider === "local") {
    const [status, chave] = await Promise.all([
      getServerStatus().catch(() => null),
      getSetting("server_api_key").catch(() => null),
    ]);
    return { baseUrl: status?.baseUrl ?? PADRAO_LOCAL, usaChave: !!chave };
  }
  const ep = await providerEndpoint(modelRef);
  return { baseUrl: ep.baseUrl, usaChave: true };
}

const FORMATOS: { id: FormatoDaRequisicao; rotulo: string }[] = [
  { id: "curl", rotulo: "cURL" },
  { id: "python", rotulo: "Python" },
  { id: "js", rotulo: "JavaScript" },
];

export default function CopyRequest({
  model,
  messages,
  params,
}: {
  model: string;
  messages: ChatMessage[];
  params: ChatParams;
}) {
  const { t } = useTranslation();

  const copiar = async (formato: FormatoDaRequisicao) => {
    try {
      const { baseUrl, usaChave } = await destino(model);
      const mensagens: ChatMessage[] =
        messages.length > 0 ? messages : [{ role: "user", content: t("chat.copyRequest.sample") }];
      const body = buildChatBody({
        model: splitModelRef(model).model,
        messages: mensagens,
        params,
        stream: false,
      });
      await navigator.clipboard.writeText(gerarRequisicao(formato, { baseUrl, body, usaChave }));
      toast({
        tone: "ok",
        message: t("chat.copyRequest.copied", { format: FORMATOS.find((f) => f.id === formato)?.rotulo }),
        duration: 3000,
      });
    } catch (e) {
      toast({ tone: "bad", message: t("chat.copyRequest.failed", { error: String(e) }), duration: 6000 });
    }
  };

  return (
    <div className="flex flex-col gap-1.5" role="group" aria-label={t("chat.copyRequest.title")}>
      <span className="text-xs text-dim">{t("chat.copyRequest.title")}</span>
      <div className="flex flex-wrap gap-1.5">
        {FORMATOS.map((f) => (
          <Button key={f.id} size="sm" onClick={() => void copiar(f.id)}>
            {f.rotulo}
          </Button>
        ))}
      </div>
      <span className="text-[10px] leading-snug text-dim">{t("chat.copyRequest.hint")}</span>
    </div>
  );
}
