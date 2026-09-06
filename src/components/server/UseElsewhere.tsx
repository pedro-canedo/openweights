// "Usar em outro app": o destino de tudo que esta tela configura.
//
// O servidor local só tem sentido quando outro programa fala com ele. Este
// card junta os dois jeitos de chegar lá, na ordem em que as pessoas os
// procuram: primeiro os agentes de código que abrem apontados para cá com um
// clique, depois — dobrado, para quem quiser — o mesmo endereço em curl e em
// Python, para qualquer outra coisa.
//
// Os exemplos já saem prontos: com chave definida, o cabeçalho de
// autorização vem preenchido; sem ela, some. Um exemplo que precisa de edição
// antes de funcionar é um exemplo que não foi lido até o fim.

import { useTranslation } from "react-i18next";
import { Card, Collapse } from "../ui/Shell";
import { CodeBlock } from "../ui/Copy";
import HarnessLauncher from "./HarnessLauncher";

export default function UseElsewhere({
  baseUrl,
  apiKey,
  model,
  loaded,
  running,
}: {
  baseUrl: string | null;
  apiKey: string;
  /** Modelo carregado no Router — é o que os harnesses recebem. */
  model: string;
  loaded: boolean;
  running: boolean;
}) {
  const { t } = useTranslation();
  const url = baseUrl ?? "http://127.0.0.1:11711";
  const nomeModelo = model || "SEU-MODELO";

  const curl = `curl ${url}/v1/chat/completions \\
  -H "Content-Type: application/json" \\${apiKey ? `\n  -H "Authorization: Bearer ${apiKey}" \\` : ""}
  -d '{"model": "${nomeModelo}", "messages": [{"role": "user", "content": "Olá!"}]}'`;

  // API Anthropic nativa do llama-server: a raiz + /v1/messages, body mínimo
  // (model, max_tokens, messages). O header de auth do lado Anthropic é o
  // x-api-key — o servidor aceita este e o Bearer.
  const curlClaude = `curl ${url}/v1/messages \\
  -H "Content-Type: application/json" \\${apiKey ? `\n  -H "x-api-key: ${apiKey}" \\` : ""}
  -d '{"model": "${nomeModelo}", "max_tokens": 512, "messages": [{"role": "user", "content": "Olá!"}]}'`;

  const python = `from openai import OpenAI

client = OpenAI(base_url="${url}/v1", api_key="${apiKey || "local"}")
resp = client.chat.completions.create(
    model="${nomeModelo}",
    messages=[{"role": "user", "content": "Olá!"}],
)
print(resp.choices[0].message.content)`;

  return (
    <>
      <Card title={t("server.use.title")} hint={t("server.use.hint")}>
        <div className="mt-3">
          <HarnessLauncher model={model} loaded={loaded} running={running} />
        </div>
      </Card>

      <Collapse
        title={t("server.exampleTitle")}
        hint={t("server.use.examplesHint")}
      >
        <div className="flex flex-col gap-3">
          <div>
            <div className="mb-1 text-[11px] text-dim">
              {t("server.use.openaiLabel")}
            </div>
            <CodeBlock code={curl} />
          </div>
          <div>
            <div className="mb-1 text-[11px] text-dim">
              {t("server.use.anthropicLabel")}
            </div>
            <CodeBlock code={curlClaude} />
          </div>
          <div>
            <div className="mb-1 text-[11px] text-dim">
              {t("server.use.pythonLabel")}
            </div>
            <CodeBlock code={python} />
          </div>
        </div>
      </Collapse>
    </>
  );
}
