// As fotos de perfil dos autores no Hub — pedidas uma por linha, buscadas em
// lote, lembradas entre sessões.
//
// Quem pergunta é cada item da lista, isoladamente; quem responde é um
// comando que aceita a lista inteira. O meio de campo é este módulo: os
// pedidos que chegam no mesmo instante viram UMA consulta, e a resposta fica
// guardada — em memória pelo resto da sessão e no `localStorage` por uma
// semana, para que reabrir o app mostre as fotos antes de a rede responder.
//
// O "este autor não tem foto" também é guardado: descobrir isso custa as
// mesmas duas requisições que descobrir a URL, e sem lembrar a ausência cada
// letra digitada na busca mandaria a lista toda de novo para a rede.

import { useEffect, useState } from "react";
import { authorAvatars } from "./api";

const CHAVE = "ow:hf-avatars";
const VALIDADE_MS = 7 * 24 * 60 * 60 * 1000;
/** Janela em que os pedidos de uma mesma renderização viram um lote só. */
const ESPERA_MS = 50;

type Guardado = { u: string | null; t: number };

const memoria = new Map<string, Guardado>();
let lido = false;

/** Traz o que a sessão anterior descobriu, descartando o que envelheceu. */
function consultar(autor: string): Guardado | undefined {
  if (!lido) {
    lido = true;
    try {
      const bruto = localStorage.getItem(CHAVE);
      const agora = Date.now();
      for (const [nome, g] of Object.entries<Guardado>(
        bruto ? JSON.parse(bruto) : {},
      )) {
        if (g && typeof g.t === "number" && agora - g.t < VALIDADE_MS) {
          memoria.set(nome, g);
        }
      }
    } catch {
      // O cache em disco é conveniência: sem ele, a rede responde de novo.
    }
  }
  return memoria.get(autor);
}

function gravar() {
  try {
    localStorage.setItem(CHAVE, JSON.stringify(Object.fromEntries(memoria)));
  } catch {
    // Cota cheia ou armazenamento bloqueado: a sessão segue com a memória.
  }
}

const fila = new Map<string, ((url: string | null) => void)[]>();
let timer: ReturnType<typeof setTimeout> | null = null;

function pedir(autor: string): Promise<string | null> {
  return new Promise((responder) => {
    const esperando = fila.get(autor);
    if (esperando) {
      esperando.push(responder);
      return;
    }
    fila.set(autor, [responder]);
    timer ??= setTimeout(despachar, ESPERA_MS);
  });
}

async function despachar() {
  timer = null;
  const lote = new Map(fila);
  fila.clear();

  let fotos: Record<string, string>;
  try {
    fotos = await authorAvatars([...lote.keys()]);
  } catch {
    // Sem rede não há foto, e as iniciais seguem no lugar. A ausência NÃO vai
    // para o cache: falha de rede não é resposta, e a próxima abertura tenta
    // de novo.
    for (const respostas of lote.values()) respostas.forEach((r) => r(null));
    return;
  }

  const t = Date.now();
  for (const [autor, respostas] of lote) {
    const u = fotos[autor] ?? null;
    memoria.set(autor, { u, t });
    respostas.forEach((r) => r(u));
  }
  gravar();
}

/** A foto do autor, ou `null` enquanto não se sabe (ou quando não há). */
export function useAuthorAvatar(author: string): string | null {
  const [url, setUrl] = useState<string | null>(
    () => (author ? consultar(author)?.u : null) ?? null,
  );

  useEffect(() => {
    if (!author) {
      setUrl(null);
      return;
    }
    const guardado = consultar(author);
    if (guardado) {
      setUrl(guardado.u);
      return;
    }
    // A troca de autor zera a foto antes de perguntar: sem isto, uma linha
    // reaproveitada pela lista mostraria por um instante a foto de outro.
    setUrl(null);
    let vivo = true;
    pedir(author).then((u) => {
      if (vivo) setUrl(u);
    });
    return () => {
      vivo = false;
    };
  }, [author]);

  return url;
}

/** O autor de um `autor/modelo` — o que a biblioteca local guarda é o par. */
export function autorDoRepo(repoId: string): string {
  return repoId.split("/")[0] ?? "";
}
