// Paridade e sanidade do site de documentação.
//
// Irmão do `i18n-parity.mjs`, e existe pelo mesmo motivo: a garantia era só
// humana. O site promete, em `contribute/index.md`, que "uma página sob `site/`
// ganha a contraparte sob `site/pt/`" — e essa promessa era mantida na base da
// disciplina. Em 24 versões publicadas, duas saíram sem tocar em documentação
// nenhuma, e ninguém foi avisado.
//
// Uma página que falta num idioma não quebra o build do VitePress: ela
// simplesmente não existe para quem lê naquela língua. Uma página fora da barra
// lateral existe e ninguém acha. Um link para `blob/main/` de um arquivo que
// carrega a versão no nome responde 200 hoje e 404 na próxima release — e
// nesse dia ninguém está olhando. Nada disso dá erro em lugar nenhum.
//
// O que este script NÃO faz: comparar o texto. Títulos são traduzidos, e é
// assim que tem de ser. O que ele compara é a FORMA — a sequência de níveis —
// que é o que monta o sumário lateral e o que o leitor usa para se orientar.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, relative } from "node:path";

const raiz = join(dirname(fileURLToPath(import.meta.url)), "..");
const site = join(raiz, "site");

// O par EN↔PT de cada página. Os slugs são traduzidos de propósito — uma URL em
// português que diz "performance" é uma URL pela metade — então não há como
// inferir o par: este mapa É o dicionário. Página nova sem entrada aqui derruba
// a regra de cobertura logo abaixo, que é exatamente o efeito desejado: criar
// página passa a ser uma decisão registrada, não um arquivo que apareceu.
const PARES = [
  ["index.md", "pt/index.md"],
  ["guide/index.md", "pt/guia/index.md"],
  ["guide/whats-new.md", "pt/guia/novidades.md"],
  ["guide/changelog.md", "pt/guia/changelog.md"],
  ["guide/install.md", "pt/guia/instalacao.md"],
  ["guide/first-run.md", "pt/guia/primeira-execucao.md"],
  ["guide/models.md", "pt/guia/modelos.md"],
  ["guide/chat.md", "pt/guia/chat.md"],
  ["guide/performance.md", "pt/guia/desempenho.md"],
  ["guide/harness.md", "pt/guia/harness.md"],
  ["guide/troubleshooting.md", "pt/guia/solucao-de-problemas.md"],
  ["integrations/local-api.md", "pt/integracoes/api-local.md"],
  ["integrations/cluster.md", "pt/integracoes/cluster.md"],
  ["integrations/providers.md", "pt/integracoes/provedores.md"],
  ["integrations/api-reference.md", "pt/integracoes/referencia-api.md"],
  ["integrations/configuration.md", "pt/integracoes/configuracao.md"],
  ["contribute/index.md", "pt/contribuir/index.md"],
  ["contribute/architecture.md", "pt/contribuir/arquitetura.md"],
];

// As duas landings usam `layout: page` e não entram na barra lateral de
// propósito: quem chega nelas veio do topo, não de dentro do guia.
const FORA_DA_SIDEBAR = new Set(["/", "/pt/"]);

// `blob/main/` só vale para caminho que não muda de nome. Um link para
// `blob/main/docs/performance-0.17.0.md` responde 200 hoje e 404 no dia em que
// a 0.18.0 renomear o arquivo. Conteúdo versionado se referencia pela TAG.
const CAMINHOS_ESTAVEIS = [
  "LICENSE", "README.md", "README.pt-BR.md", "CONTRIBUTING.md",
  "SECURITY.md", "CODE_OF_CONDUCT.md", "CHANGELOG.md", "src", "src-tauri",
  "scripts", "docs/graph.md",
];

const problemas = [];
const falhar = (msg) => problemas.push(msg);

function listarMd(dir) {
  const saida = [];
  for (const nome of readdirSync(dir)) {
    if (nome === "node_modules" || nome === ".vitepress" || nome === "dist") continue;
    const caminho = join(dir, nome);
    if (statSync(caminho).isDirectory()) saida.push(...listarMd(caminho));
    else if (nome.endsWith(".md")) saida.push(caminho);
  }
  return saida;
}

const naArvore = new Set(
  listarMd(site).map((p) => relative(site, p).split("\\").join("/")),
);
const noMapa = new Set(PARES.flat());

// ── 1. cobertura ───────────────────────────────────────────────────────────
for (const arquivo of [...naArvore].sort()) {
  if (!noMapa.has(arquivo)) {
    falhar(`site/${arquivo} existe e não está em PARES — declare o par dela.`);
  }
}
for (const arquivo of [...noMapa].sort()) {
  if (!naArvore.has(arquivo)) {
    falhar(`PARES aponta para site/${arquivo}, que não existe.`);
  }
}

// ── 2. forma dos títulos ───────────────────────────────────────────────────
// Blocos de código e frontmatter ficam de fora: um `# comentário` dentro de um
// bloco bash seria lido como título e o script mentiria com toda a confiança.
function titulos(texto) {
  const linhas = texto.split(/\r?\n/);
  const saida = [];
  let emCerca = false;
  let i = 0;
  if (linhas[0]?.trim() === "---") {
    const fecha = linhas.findIndex((l, n) => n > 0 && l.trim() === "---");
    if (fecha > 0) i = fecha + 1;
  }
  for (; i < linhas.length; i++) {
    if (/^\s*(```|~~~)/.test(linhas[i])) { emCerca = !emCerca; continue; }
    if (emCerca) continue;
    const m = /^(#{2,3})\s+(.+?)\s*$/.exec(linhas[i]);
    if (m) saida.push({ nivel: m[1].length, texto: m[2], linha: i + 1 });
  }
  return saida;
}

const conteudo = new Map(
  [...naArvore].map((a) => [a, readFileSync(join(site, a), "utf8")]),
);

// Uma página pode declarar `paridade: livre` no frontmatter quando a estrutura
// realmente diverge por bom motivo. A isenção aparece na saída de sucesso toda
// rodada — isenção silenciosa é buraco; isenção que alguém lê é decisão.
const isenta = (texto) => /^---\n[\s\S]*?^paridade:\s*livre\s*$/m.test(texto);

let comparados = 0;
const isencoes = [];
for (const [en, pt] of PARES) {
  if (!naArvore.has(en) || !naArvore.has(pt)) continue;
  if (isenta(conteudo.get(en))) { isencoes.push(en); continue; }

  const tEn = titulos(conteudo.get(en));
  const tPt = titulos(conteudo.get(pt));
  comparados += tEn.length;

  const n = Math.max(tEn.length, tPt.length);
  for (let i = 0; i < n; i++) {
    const a = tEn[i], b = tPt[i];
    if (a && b && a.nivel === b.nivel) continue;
    const mostra = (h) =>
      h ? `${"#".repeat(h.nivel)} ${h.texto}  (linha ${h.linha})` : "(não existe)";
    falhar(
      `site/${en} e site/${pt} divergem no título ${i + 1}:\n` +
        `    EN  ${mostra(a)}\n` +
        `    PT  ${mostra(b)}\n` +
        `    EN tem ${tEn.length} título(s), PT tem ${tPt.length}.`,
    );
    break;
  }
}

// ── 3. barra lateral ───────────────────────────────────────────────────────
const config = readFileSync(join(site, ".vitepress", "config.ts"), "utf8");

function linksDaSidebar(nome) {
  const inicio = config.indexOf(`const ${nome} = [`);
  if (inicio < 0) {
    falhar(`não achei "const ${nome}" em site/.vitepress/config.ts`);
    return new Set();
  }
  const bloco = config.slice(inicio, config.indexOf("\n];", inicio));
  return new Set([...bloco.matchAll(/link:\s*"([^"]+)"/g)].map((m) => m[1]));
}

const naSidebar = new Set([
  ...linksDaSidebar("enSidebar"),
  ...linksDaSidebar("ptSidebar"),
]);

const paraLink = (p) => "/" + p.replace(/\.md$/, "").replace(/(^|\/)index$/, "$1");

for (const arquivo of [...naArvore].sort()) {
  const link = paraLink(arquivo);
  if (FORA_DA_SIDEBAR.has(link)) continue;
  if (!naSidebar.has(link)) {
    falhar(`site/${arquivo} não aparece na barra lateral — página que ninguém acha.`);
  }
}
for (const link of [...naSidebar].sort()) {
  const candidatos = [
    `${link.replace(/^\//, "")}.md`,
    `${link.replace(/^\//, "")}index.md`,
  ];
  if (!candidatos.some((c) => naArvore.has(c))) {
    falhar(`a barra lateral aponta para ${link}, que não tem arquivo.`);
  }
}

// ── 4. as versões do "novidades" batem entre os idiomas ────────────────────
const versoesDe = (arquivo) =>
  titulos(conteudo.get(arquivo) ?? "")
    .filter((h) => h.nivel === 2)
    .map((h) => /^(\d+\.\d+\.\d+)\b/.exec(h.texto)?.[1])
    .filter(Boolean);

const vEn = versoesDe("guide/whats-new.md");
const vPt = versoesDe("pt/guia/novidades.md");
if (vEn.join(",") !== vPt.join(",")) {
  falhar(
    `as versões do "novidades" não batem:\n` +
      `    EN  ${vEn.join(", ") || "(nenhuma)"}\n` +
      `    PT  ${vPt.join(", ") || "(nenhuma)"}`,
  );
}

// ── 5. link versionado ─────────────────────────────────────────────────────
for (const [arquivo, texto] of conteudo) {
  for (const m of texto.matchAll(/blob\/main\/([^\s)"'#]+)/g)) {
    const alvo = m[1];
    const estavel = CAMINHOS_ESTAVEIS.some(
      (p) => alvo === p || alvo.startsWith(`${p}/`),
    );
    if (!estavel) {
      falhar(
        `site/${arquivo}: link para blob/main/${alvo}\n` +
          `    Use blob/v<versão>/${alvo} — o arquivo carrega a versão no nome e\n` +
          `    o link em main vira 404 na próxima release.`,
      );
    }
  }
}

// ── veredito ───────────────────────────────────────────────────────────────
if (problemas.length > 0) {
  console.error(`\n${problemas.length} problema(s) na documentação:\n`);
  for (const p of problemas) console.error(`  ${p}\n`);
  console.error(
    "Escreva a página nos DOIS idiomas no mesmo commit. Um rascunho em português\n" +
      "é melhor que uma página pela metade — traduzir depois vira dívida que\n" +
      "ninguém paga.",
  );
  process.exit(1);
}

const extra = isencoes.length ? `, ${isencoes.length} isenta(s) de forma` : "";
console.log(
  `docs: ${PARES.length} pares EN↔PT, ${comparados} títulos comparados, ` +
    `${naSidebar.size} links de barra lateral, ${vEn.length} versões no novidades${extra}.`,
);
