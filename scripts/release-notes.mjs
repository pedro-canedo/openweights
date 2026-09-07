// As notas de uma versão, escritas uma vez.
//
// Existe porque o mesmo texto era escrito três vezes: em `docs/releases/`, no
// `releaseBody` fixo do `release.yml` e à mão no site. Três cópias combinam no
// dia em que nascem e divergem na terceira release — e a que o usuário lê é a
// que ninguém corrigiu.
//
// A fonte é `docs/releases/<versão>.md`: um arquivo, bilíngue, com frontmatter.
// Daqui saem o corpo do release no GitHub, a mensagem da tag assinada e o
// CHANGELOG.md. O "Novidades" do site continua escrito à mão de propósito: ele
// é prosa que explica o PORQUÊ de cada mudança, e gerar isso de uma lista de
// bullets trocaria a melhor página do site por um despejo.
//
//   node scripts/release-notes.mjs --body [versão]        corpo do release
//   node scripts/release-notes.mjs --tag [versão]         mensagem da tag
//   node scripts/release-notes.mjs --changelog --write    regrava CHANGELOG.md
//   node scripts/release-notes.mjs --changelog --check    confere sem gravar

import { readFileSync, writeFileSync, readdirSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const raiz = join(dirname(fileURLToPath(import.meta.url)), "..");
const pastaReleases = join(raiz, "docs", "releases");

// Abaixo desta linha o CHANGELOG é histórico recuperado dos releases antigos,
// que não têm arquivo em docs/releases/. O gerador copia verbatim.
const SENTINELA = "<!-- gerado-ate-aqui:";

export function versaoDoPacote() {
  return JSON.parse(readFileSync(join(raiz, "package.json"), "utf8")).version;
}

// Frontmatter mínimo (`data:` e `destaque:`) para o gerador não ter de adivinhar
// a data nem o subtítulo. Sem dependência de YAML: são duas chaves de uma linha.
function frontmatter(texto) {
  const m = /^---\n([\s\S]*?)\n---\n/.exec(texto);
  if (!m) return [{}, texto];
  const campos = {};
  for (const linha of m[1].split("\n")) {
    const par = /^(\w+):\s*(.*)$/.exec(linha);
    if (par) campos[par[1]] = par[2].trim();
  }
  return [campos, texto.slice(m[0].length)];
}

export function lerNotas(versao) {
  const caminho = join(pastaReleases, `${versao}.md`);
  if (!existsSync(caminho)) {
    throw new Error(
      `docs/releases/${versao}.md não existe.\n` +
        `Toda versão tem notas próprias — copie a estrutura de outra em docs/releases/.`,
    );
  }
  const [campos, corpo] = frontmatter(readFileSync(caminho, "utf8"));
  // Fatia por linha de `## `: com regex, o `$` do multiline casa no fim da
  // PRIMEIRA linha e a seção volta vazia — erro silencioso, porque o resultado
  // é uma string, só que sem nada dentro.
  const secao = (nome) => {
    const linhas = corpo.split("\n");
    const inicio = linhas.findIndex((l) => l.trim() === `## ${nome}`);
    if (inicio < 0) return "";
    let fim = linhas.length;
    for (let i = inicio + 1; i < linhas.length; i++) {
      if (/^## /.test(linhas[i])) { fim = i; break; }
    }
    return linhas.slice(inicio + 1, fim).join("\n").trim();
  };
  return {
    versao,
    data: campos.data ?? "",
    destaque: campos.destaque ?? "",
    portugues: secao("Português"),
    ingles: secao("English"),
    // Uma versão pode trazer instalação própria; sem isso, vale a partilhada.
    instalacao: secao("Installation") || instalacaoPadrao(),
  };
}

function instalacaoPadrao() {
  const caminho = join(pastaReleases, "_instalacao.md");
  if (!existsSync(caminho)) return "";
  return readFileSync(caminho, "utf8").replace(/^## Installation\n/, "").trim();
}

export function versoesComNotas() {
  return readdirSync(pastaReleases)
    .filter((n) => n.endsWith(".md") && !n.startsWith("_"))
    .map((n) => n.replace(/\.md$/, ""))
    .sort(porSemverDecrescente);
}

const porSemverDecrescente = (a, b) => {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) if (pa[i] !== pb[i]) return pb[i] - pa[i];
  return 0;
};

// O corpo do release vai em inglês: é o público do GitHub. O português mora no
// site, linkado logo abaixo.
function corpoDoRelease(n) {
  const site = "https://pedro-canedo.github.io/openweights";
  return [
    n.ingles,
    "",
    "## Installation",
    "",
    n.instalacao,
    "",
    `Notas em português: ${site}/pt/guia/novidades`,
    `Todas as versões: https://github.com/pedro-canedo/openweights/blob/v${n.versao}/CHANGELOG.md`,
  ].join("\n");
}

function mensagemDaTag(n) {
  const titulo = n.destaque
    ? `OpenWeights ${n.versao}: ${n.destaque}`
    : `OpenWeights ${n.versao}`;
  return [titulo, "", n.portugues, "", "---", "", n.ingles].join("\n");
}

function montarChangelog() {
  const atual = existsSync(join(raiz, "CHANGELOG.md"))
    ? readFileSync(join(raiz, "CHANGELOG.md"), "utf8")
    : "";
  const corte = atual.indexOf(SENTINELA);
  if (corte < 0) {
    throw new Error(
      `CHANGELOG.md não tem a marca "${SENTINELA}".\n` +
        `Ela separa o que é gerado do histórico escrito à mão — sem ela o gerador\n` +
        `não sabe onde parar e apagaria as versões antigas.`,
    );
  }
  const cabecalho = atual.slice(0, atual.indexOf("\n## ["));
  const historico = atual.slice(corte);

  const secoes = versoesComNotas().map((v) => {
    const n = lerNotas(v);
    return `## [${n.versao}] — ${n.data}\n\n${n.ingles}\n`;
  });

  return `${cabecalho}\n${secoes.join("\n")}\n${historico}`;
}

const args = process.argv.slice(2);
const flag = (nome) => args.includes(nome);
const versaoPedida = args.find((a) => /^\d+\.\d+\.\d+$/.test(a)) ?? versaoDoPacote();

if (flag("--body")) {
  process.stdout.write(corpoDoRelease(lerNotas(versaoPedida)) + "\n");
} else if (flag("--tag")) {
  process.stdout.write(mensagemDaTag(lerNotas(versaoPedida)) + "\n");
} else if (flag("--changelog")) {
  const montado = montarChangelog();
  if (flag("--write")) {
    writeFileSync(join(raiz, "CHANGELOG.md"), montado);
    console.log(`CHANGELOG.md regravado a partir de docs/releases/.`);
  } else {
    const disco = readFileSync(join(raiz, "CHANGELOG.md"), "utf8");
    if (disco !== montado) {
      console.error(
        "CHANGELOG.md não corresponde a docs/releases/.\n" +
          "Rode: node scripts/release-notes.mjs --changelog --write",
      );
      process.exit(1);
    }
    console.log("CHANGELOG.md em dia com docs/releases/.");
  }
} else {
  console.error("Use --body, --tag ou --changelog (--write | --check).");
  process.exit(2);
}
