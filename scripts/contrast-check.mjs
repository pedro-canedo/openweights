#!/usr/bin/env node
// Confere o contraste dos tokens de cor (src/styles.css) nos dois temas.
//
// Os tokens são a única fonte das cores de texto, borda e foco da interface;
// um valor novo que pareça bom no monitor de quem editou pode ficar ilegível
// no tema ao lado. Este portão mede a razão WCAG 2.x de cada par que a
// interface realmente usa e falha o build quando algum fica abaixo do piso:
// 4,5:1 para texto, 3:1 para borda de controle e anel de foco.
//
// Só a stdlib do Node, como o i18n-parity e o docs-parity.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const CSS = fileURLToPath(new URL("../src/styles.css", import.meta.url));

const TEXTO = 4.5;
const CONTROLE = 3;
const FUNDOS = ["bg", "panel", "panel2"];

/**
 * Pares [frente, fundos, piso, uso]. "#ffffff" é o texto dos botões cheios;
 * "cor@15" é o fundo tingido dos selos (`bg-ok/15 text-ok`), a cor a 15% sobre
 * o painel.
 */
const PARES = [
  ["ink", FUNDOS, TEXTO, "texto principal"],
  ["dim", FUNDOS, TEXTO, "texto secundário"],
  ["ok", [...FUNDOS, "ok@15"], TEXTO, "texto de sucesso"],
  ["warn", [...FUNDOS, "warn@15"], TEXTO, "texto de aviso"],
  ["bad", [...FUNDOS, "bad@15"], TEXTO, "texto de erro"],
  ["accent-ink", [...FUNDOS, "accent@15"], TEXTO, "texto na cor da marca"],
  ["#ffffff", ["accent-fill"], TEXTO, "rótulo de botão cheio"],
  ["edge-strong", FUNDOS, CONTROLE, "borda de campo"],
  ["focus", FUNDOS, CONTROLE, "anel de foco"],
];

function bloco(css, seletor) {
  const inicio = css.indexOf(`${seletor} {`);
  if (inicio < 0) throw new Error(`bloco ${seletor} não encontrado em styles.css`);
  const fim = css.indexOf("}", inicio);
  const vars = {};
  for (const m of css.slice(inicio, fim).matchAll(/--lr-([\w-]+):\s*(#[0-9a-fA-F]{6})\b/g)) {
    vars[m[1]] = m[2].toLowerCase();
  }
  return vars;
}

/** Mistura `frente` a `peso` sobre `fundo`, canal a canal (aproxima o color-mix). */
function misturar(frente, fundo, peso) {
  const canal = (hex, i) => parseInt(hex.slice(i, i + 2), 16);
  return (
    "#" +
    [1, 3, 5]
      .map((i) => Math.round(canal(frente, i) * peso + canal(fundo, i) * (1 - peso)))
      .map((v) => v.toString(16).padStart(2, "0"))
      .join("")
  );
}

function luminancia(hex) {
  const canais = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
  const [r, g, b] = canais.map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function razao(a, b) {
  const [l1, l2] = [luminancia(a), luminancia(b)].sort((x, y) => y - x);
  return (l1 + 0.05) / (l2 + 0.05);
}

const css = readFileSync(CSS, "utf8");
const escuro = bloco(css, ":root");
// O claro só redefine o que muda; o resto vem do escuro.
const claro = { ...escuro, ...bloco(css, ':root[data-theme="light"]') };

const falhas = [];
for (const [nome, vars] of [["escuro", escuro], ["claro", claro]]) {
  const cor = (token) => {
    if (token.startsWith("#")) return token;
    const tinta = token.match(/^([\w-]+)@(\d+)$/);
    if (tinta) return misturar(cor(tinta[1]), cor("panel"), Number(tinta[2]) / 100);
    const v = vars[token];
    if (!v) throw new Error(`tema ${nome}: --lr-${token} não está definido`);
    return v;
  };
  for (const [frente, fundos, piso, uso] of PARES) {
    for (const fundo of fundos) {
      const r = razao(cor(frente), cor(fundo));
      if (r < piso) {
        falhas.push(`  ${nome}: ${frente} sobre ${fundo} = ${r.toFixed(2)}:1 (mínimo ${piso}:1, ${uso})`);
      }
    }
  }
}

if (falhas.length) {
  console.error(`contrast-check: ${falhas.length} par(es) abaixo do piso WCAG AA:`);
  console.error(falhas.join("\n"));
  console.error("Ajuste os valores --lr-* em src/styles.css.");
  process.exit(1);
}
const total = PARES.reduce((n, [, fundos]) => n + fundos.length, 0) * 2;
console.log(`contrast-check: ${total} pares de tokens dentro do piso WCAG AA nos dois temas.`);
