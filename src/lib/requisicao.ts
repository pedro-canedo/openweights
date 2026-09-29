// "Copiar requisição": a chamada do chat como comando de curl, código em Python
// ou em JavaScript. O corpo vem de `buildChatBody` — o mesmo que o chat envia —,
// só sem streaming, e a chave de API nunca entra: vai a variável de ambiente.

import { aspas } from "./comandoShell";

export type FormatoDaRequisicao = "curl" | "python" | "js";

export interface DadosDaRequisicao {
  /** A raiz do servidor, sem `/v1`. */
  baseUrl: string;
  /** O corpo de `buildChatBody(..., stream: false)`. */
  body: Record<string, unknown>;
  /** A fonte pede chave: entra o cabeçalho, com a variável de ambiente. */
  usaChave: boolean;
}

/** Imagem em base64 não vai para a área de transferência: vira um marcador. */
export function semImagensEmBase64<T>(valor: T): T {
  if (Array.isArray(valor)) return valor.map(semImagensEmBase64) as T;
  if (valor && typeof valor === "object") {
    const saida: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(valor)) {
      saida[k] =
        k === "url" && typeof v === "string" && v.startsWith("data:") ? "<imagem em base64>" : semImagensEmBase64(v);
    }
    return saida as T;
  }
  return valor;
}

/** Um valor JSON como literal do Python (`true` → `True`, `null` → `None`). */
export function literalPython(v: unknown, recuo = 0): string {
  const pad = "    ".repeat(recuo + 1);
  const fim = "    ".repeat(recuo);
  if (v === null || v === undefined) return "None";
  if (typeof v === "boolean") return v ? "True" : "False";
  if (typeof v === "number") return String(v);
  if (typeof v === "string") return JSON.stringify(v);
  if (Array.isArray(v)) {
    if (v.length === 0) return "[]";
    return `[\n${v.map((x) => `${pad}${literalPython(x, recuo + 1)},`).join("\n")}\n${fim}]`;
  }
  const entradas = Object.entries(v as Record<string, unknown>);
  if (entradas.length === 0) return "{}";
  return `{\n${entradas.map(([k, x]) => `${pad}${JSON.stringify(k)}: ${literalPython(x, recuo + 1)},`).join("\n")}\n${fim}}`;
}

/** O que o SDK da OpenAI aceita como argumento; o resto vai em `extra_body`. */
const ARGUMENTOS_DO_SDK = new Set([
  "model",
  "messages",
  "temperature",
  "top_p",
  "max_tokens",
  "presence_penalty",
  "frequency_penalty",
  "seed",
  "stop",
  "response_format",
  "stream",
]);

export function gerarRequisicao(formato: FormatoDaRequisicao, d: DadosDaRequisicao): string {
  const url = `${d.baseUrl.replace(/\/+$/, "")}/v1/chat/completions`;
  const body = semImagensEmBase64(d.body);
  if (formato === "curl") {
    const cabecalhos = ['-H "Content-Type: application/json"'];
    if (d.usaChave) cabecalhos.push('-H "Authorization: Bearer $OPENWEIGHTS_API_KEY"');
    return [
      `curl ${aspas(url)}`,
      ...cabecalhos.map((h) => `  ${h}`),
      `  -d ${aspas(JSON.stringify(body, null, 2))}`,
    ].join(" \\\n");
  }
  if (formato === "python") {
    const padrao: Record<string, unknown> = {};
    const extra: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(body)) (ARGUMENTOS_DO_SDK.has(k) ? padrao : extra)[k] = v;
    const args = Object.entries(padrao)
      .map(([k, v]) => `    ${k}=${literalPython(v, 1)},`)
      .join("\n");
    const extras =
      Object.keys(extra).length > 0 ? `\n    extra_body=${literalPython(extra, 1)},` : "";
    return `import os
from openai import OpenAI

client = OpenAI(
    base_url="${d.baseUrl.replace(/\/+$/, "")}/v1",
    api_key=${d.usaChave ? 'os.environ["OPENWEIGHTS_API_KEY"]' : '"local"'},
)
resp = client.chat.completions.create(
${args}${extras}
)
print(resp.choices[0].message.content)`;
  }
  const cabecalhos = ['"Content-Type": "application/json"'];
  if (d.usaChave) cabecalhos.push("Authorization: `Bearer ${process.env.OPENWEIGHTS_API_KEY}`");
  return `const resp = await fetch(${JSON.stringify(url)}, {
  method: "POST",
  headers: { ${cabecalhos.join(", ")} },
  body: JSON.stringify(${JSON.stringify(body, null, 2).replace(/\n/g, "\n  ")}),
});
const data = await resp.json();
console.log(data.choices[0].message.content);`;
}
