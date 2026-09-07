// As versões, lidas de `docs/releases/` na hora do build.
//
// A página de changelog não guarda cópia nenhuma: ela renderiza o que este
// carregador leu da fonte única. Foi assim que o mesmo texto deixou de ser
// escrito três vezes — o corpo do release, o CHANGELOG.md e esta página saem
// todos de `docs/releases/<versão>.md`.
//
// `watch` faz o VitePress reconstruir quando uma nota muda; sem isso, escrever
// uma release nova durante o `npm run dev` não apareceria na tela.

import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join } from "node:path";
import { createMarkdownRenderer } from "vitepress";

const pasta = join(process.cwd(), "..", "docs", "releases");

export interface Versao {
  versao: string;
  data: string;
  destaque: string;
  /** Já em HTML: converter aqui evita arrastar um parser de markdown para o
   *  bundle do cliente só para renderizar texto que o build já leu. */
  portugues: string;
  ingles: string;
}

declare const data: Versao[];
export { data };

function frontmatter(texto: string): [Record<string, string>, string] {
  const m = /^---\n([\s\S]*?)\n---\n/.exec(texto);
  if (!m) return [{}, texto];
  const campos: Record<string, string> = {};
  for (const linha of m[1].split("\n")) {
    const par = /^(\w+):\s*(.*)$/.exec(linha);
    if (par) campos[par[1]] = par[2].trim();
  }
  return [campos, texto.slice(m[0].length)];
}

// Fatia por linha de `## `: com regex multilinha o `$` casa no fim da PRIMEIRA
// linha e a seção volta vazia — erro silencioso, porque o resultado ainda é
// uma string.
function secao(corpo: string, nome: string): string {
  const linhas = corpo.split("\n");
  const inicio = linhas.findIndex((l) => l.trim() === `## ${nome}`);
  if (inicio < 0) return "";
  let fim = linhas.length;
  for (let i = inicio + 1; i < linhas.length; i++) {
    if (/^## /.test(linhas[i])) { fim = i; break; }
  }
  return linhas.slice(inicio + 1, fim).join("\n").trim();
}

const porSemverDecrescente = (a: string, b: string) => {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) if (pa[i] !== pb[i]) return pb[i] - pa[i];
  return 0;
};

export default {
  watch: ["../docs/releases/*.md"],
  async load(): Promise<Versao[]> {
    if (!existsSync(pasta)) return [];
    // O renderer do próprio VitePress: o changelog fica com o mesmo realce de
    // código e o mesmo tratamento de link do resto do site, sem dependência nova.
    const md = await createMarkdownRenderer(process.cwd());
    return readdirSync(pasta)
      .filter((n) => n.endsWith(".md") && !n.startsWith("_"))
      .map((n) => n.replace(/\.md$/, ""))
      .sort(porSemverDecrescente)
      .map((versao) => {
        const [campos, corpo] = frontmatter(
          readFileSync(join(pasta, `${versao}.md`), "utf8"),
        );
        const pt = secao(corpo, "Português");
        const en = secao(corpo, "English");
        return {
          versao,
          data: campos.data ?? "",
          destaque: campos.destaque ?? "",
          portugues: pt ? md.render(pt) : "",
          ingles: en ? md.render(en) : "",
        };
      });
  },
};
