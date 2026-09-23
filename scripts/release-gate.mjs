// Portão de release: nada sai sem a documentação da versão.
//
// Em 24 versões publicadas, a 0.14.0 e a 0.15.0 saíram sem tocar em
// documentação nenhuma, e nada avisou. A garantia era a memória de quem
// lançava, num dia em que já se fez suíte, bump, commit e push — o pior
// momento possível para depender de memória.
//
// Roda ANTES da matriz de build, de propósito: uma tag com versão desalinhada
// falha em segundos, sem gastar três runners e sem deixar um rascunho sujo no
// GitHub para alguém apagar à mão.
//
// Não para no primeiro erro. Cada rodada custa apagar e recriar uma tag
// ASSINADA — mostrar um problema de cada vez sairia caríssimo.
//
//   node scripts/release-gate.mjs          usa a tag (CI) ou o package.json

import { readFileSync, existsSync, appendFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const raiz = join(dirname(fileURLToPath(import.meta.url)), "..");
const ler = (...p) => readFileSync(join(raiz, ...p), "utf8");

// O `workflow_dispatch` do release.yml ensaia o empacotamento sem tag, com o
// rascunho descartável `v0.0.0-dev`. O ensaio continua checando documentação:
// pular tudo faria o ensaio deixar de ensaiar.
const tipoDoRef = process.env.GITHUB_REF_TYPE;
const tag = tipoDoRef === "tag" ? process.env.GITHUB_REF_NAME : null;
const ensaio = !tag || tag === "v0.0.0-dev";

const versaoPackage = JSON.parse(ler("package.json")).version;
const versao = tag && !ensaio ? tag.replace(/^v/, "") : versaoPackage;

const checagens = [];
const ok = (o) => checagens.push({ ok: true, o });
const nao = (o, comoResolver) => checagens.push({ ok: false, o, comoResolver });

// ── as três declarações de versão ──────────────────────────────────────────
const versaoTauri = JSON.parse(ler("src-tauri", "tauri.conf.json")).version;
const versaoCargo = /\[workspace\.package\][\s\S]*?^\s*version\s*=\s*"([^"]+)"/m
  .exec(ler("src-tauri", "Cargo.toml"))?.[1];

const tresIguais =
  versaoPackage === versaoTauri && versaoPackage === versaoCargo;
if (tresIguais) {
  ok(`package.json / tauri.conf.json / Cargo.toml alinhados em ${versaoPackage}`);
} else {
  nao(
    `versões desalinhadas: package.json=${versaoPackage}, ` +
      `tauri.conf.json=${versaoTauri}, Cargo.toml=${versaoCargo}`,
    "alinhe as três antes de lançar (e regenere o src-tauri/Cargo.lock)",
  );
}

// ── a tag bate com o que o projeto diz que é ───────────────────────────────
if (ensaio) {
  ok(`ensaio sem tag — conferindo a documentação da ${versao}`);
} else if (versaoPackage === versao) {
  ok(`tag ${tag} bate com a versão declarada`);
} else {
  nao(
    `a tag diz ${versao} e o projeto diz ${versaoPackage}`,
    `apague a tag, corrija o bump e crie de novo — ou tagueie v${versaoPackage}`,
  );
}

// ── as notas da versão ─────────────────────────────────────────────────────
const notas = join("docs", "releases", `${versao}.md`);
if (!existsSync(join(raiz, notas))) {
  nao(
    `${notas} não existe`,
    `crie ${notas} — copie a estrutura de outra em docs/releases/`,
  );
} else {
  const texto = ler("docs", "releases", `${versao}.md`);
  const faltando = ["## Português", "## English"].filter(
    (s) => !texto.split("\n").some((l) => l.trim() === s),
  );
  if (faltando.length) {
    nao(
      `${notas} não tem ${faltando.join(" nem ")}`,
      "as notas são bilíngues no mesmo arquivo",
    );
  } else {
    ok(`${notas} tem as duas línguas`);
  }
}

// ── o changelog ────────────────────────────────────────────────────────────
const changelog = existsSync(join(raiz, "CHANGELOG.md")) ? ler("CHANGELOG.md") : "";
if (new RegExp(`^## \\[${versao.replace(/\./g, "\\.")}\\]`, "m").test(changelog)) {
  ok(`CHANGELOG.md tem a entrada da ${versao}`);
} else {
  nao(
    `CHANGELOG.md não tem "## [${versao}]"`,
    "rode: node scripts/release-notes.mjs --changelog --write",
  );
}

// ── o "novidades" do site, nos dois idiomas ────────────────────────────────
// A prosa continua humana; o que a máquina cobra é a presença da seção.
for (const [rotulo, caminho] of [
  ["inglês", "site/guide/whats-new.md"],
  ["português", "site/pt/guia/novidades.md"],
]) {
  const texto = existsSync(join(raiz, caminho)) ? ler(caminho) : "";
  if (new RegExp(`^## ${versao.replace(/\./g, "\\.")}\\b`, "m").test(texto)) {
    ok(`${caminho} conta a ${versao}`);
  } else {
    nao(
      `${caminho} não tem "## ${versao}"`,
      `escreva em ${rotulo} o que mudou nesta versão`,
    );
  }
}

// ── o runtime do AgenticOw ─────────────────────────────────────────────────
// O pin vai dentro do binário: um pins.json vazio ou de outra release faria o
// AgenticOw dizer "não disponível nesta máquina" a todo mundo, sem erro nenhum
// no build. E a revisão e a tag do pin têm de ser as que o workflow compila.
{
  const pins = JSON.parse(ler("src-tauri", "crates", "agenticow", "pins.json"));
  const workflow = ler(".github", "workflows", "agenticow-runtime.yml");
  const doWorkflow = (nome) =>
    new RegExp(`^\\s*${nome}:\\s*(\\S+)`, "m").exec(workflow)?.[1];
  const problemas = [];
  for (const alvo of ["linux-x64", "win32-x64", "darwin-arm64"]) {
    const a = pins.assets?.[alvo];
    if (!a || !/^[0-9a-f]{64}$/.test(a.sha256 ?? "") || !(a.size > 0)) {
      problemas.push(`sem pacote válido para ${alvo}`);
    }
  }
  if (pins.tag !== doWorkflow("AGENTICOW_TAG")) {
    problemas.push(`tag ${pins.tag}, o workflow publica ${doWorkflow("AGENTICOW_TAG")}`);
  }
  if (pins.revision !== doWorkflow("AGENTICOW_REVISION")) {
    problemas.push(`revisão ${pins.revision}, o workflow compila ${doWorkflow("AGENTICOW_REVISION")}`);
  }
  if (problemas.length) {
    nao(
      `pins.json do AgenticOw: ${problemas.join("; ")}`,
      "rode o agenticow-runtime.yml com publish=true e copie o pins.json da release " +
        "para src-tauri/crates/agenticow/pins.json",
    );
  } else {
    ok(`pins.json do AgenticOw fixa ${pins.tag} (${Object.keys(pins.assets).join(", ")})`);
  }
}

// ── relatório ──────────────────────────────────────────────────────────────
const reprovadas = checagens.filter((c) => !c.ok);
const linhas = [
  "",
  `Portão de release — versão ${versao}${tag ? ` (tag ${tag})` : " (sem tag)"}`,
  "",
  ...checagens.map((c) => `  ${c.ok ? "ok   " : "FALHA"} ${c.o}`),
  "",
];

if (reprovadas.length) {
  linhas.push(
    `${reprovadas.length} checagem(ns) reprovada(s). O release não sai assim.`,
    "",
    "Para resolver, no mesmo commit da versão:",
    ...reprovadas.map((c, i) => `  ${i + 1}. ${c.comoResolver}`),
    "",
    "Rode `node scripts/release-gate.mjs` ANTES de criar a tag e isso não",
    "acontece com uma tag assinada já empurrada.",
  );
}

const relatorio = linhas.join("\n");
console[reprovadas.length ? "error" : "log"](relatorio);

// O motivo aparece na página do workflow, sem precisar abrir o log.
if (process.env.GITHUB_STEP_SUMMARY) {
  appendFileSync(
    process.env.GITHUB_STEP_SUMMARY,
    "```\n" + relatorio.trim() + "\n```\n",
  );
}
if (process.env.GITHUB_OUTPUT) {
  appendFileSync(process.env.GITHUB_OUTPUT, `versao=${versao}\n`);
}

process.exit(reprovadas.length ? 1 : 0);
