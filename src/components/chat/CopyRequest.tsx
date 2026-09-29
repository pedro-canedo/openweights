// "Copiar requisição": o que o chat mandaria agora, como curl, Python ou
// JavaScript. A chave de API nunca é copiada — vai como variável de ambiente.

import { useTranslation } from "react-i18next";
import { Button } from "../ui/Button";
import { toast } from "../ui/Toast";
import { getServerStatus, getSetting } from "../../lib/api";
import { buildChatBody, type ChatMessage } from "../../lib/llama";
import { providerEndpoint, splitModelRef } from "../../lib/providers";
import { chatReasoningEffort } from "../../lib/jev";
import { listLoadedModels, matchServerModel, visionModelFor } from "../../lib/serverSession";
import { gerarRequisicao, type FormatoDaRequisicao } from "../../lib/requisicao";
import type { ChatParams } from "../../lib/types";

const PADRAO_LOCAL = "http://127.0.0.1:11711";

interface Destino {
  baseUrl: string;
  usaChave: boolean;
  /** O nome do modelo como o servidor o conhece. */
  modelo: string;
  /** O esforço no vocabulário do template (o Bonsai 2 só aceita `xhigh`). */
  templateEffort: string | null;
}

/**
 * Para onde a requisição iria, com o que o envio real também resolve: o id do
 * modelo no Router e o nível de esforço que o template aceita. Servidor
 * parado: fica o nome do seletor. (A decisão do Jev, quando ligada, ajusta o
 * esforço por mensagem e não entra aqui.)
 */
async function destino(modelRef: string, messages: ChatMessage[], params: ChatParams): Promise<Destino> {
  const { provider, model } = splitModelRef(modelRef);
  if (provider !== "local") {
    const ep = await providerEndpoint(modelRef);
    return { baseUrl: ep.baseUrl, usaChave: true, modelo: model, templateEffort: null };
  }
  const [status, chave] = await Promise.all([
    getServerStatus().catch(() => null),
    getSetting("server_api_key").catch(() => null),
  ]);
  const baseUrl = status?.baseUrl ?? PADRAO_LOCAL;
  let modelo = model;
  if (status?.running) {
    try {
      const carregados = await listLoadedModels(baseUrl, chave ? { Authorization: `Bearer ${chave}` } : {});
      const imagem = messages.some((m) => Array.isArray(m.content) && m.content.some((p) => p.type === "image_url"));
      modelo = matchServerModel(imagem ? visionModelFor(modelRef, carregados) : modelRef, carregados);
    } catch {
      /* sem a lista, vale o nome do seletor */
    }
  }
  const templateEffort = params.effort ? await chatReasoningEffort(model, params.effort) : null;
  return { baseUrl, usaChave: !!chave, modelo, templateEffort };
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
      const mensagens: ChatMessage[] =
        messages.length > 0 ? messages : [{ role: "user", content: t("chat.copyRequest.sample") }];
      const { baseUrl, usaChave, modelo, templateEffort } = await destino(model, mensagens, params);
      const body = buildChatBody({ model: modelo, messages: mensagens, params, templateEffort, stream: false });
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
