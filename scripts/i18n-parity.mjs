// Paridade entre os arquivos de tradução.
//
// Existe porque a garantia era só humana: um checkbox no template de PR. Uma
// chave que existe em um idioma e falta no outro não quebra o build nem gera
// erro em tempo de execução — ela cai no fallback em inglês, ou renderiza a
// própria chave no meio da tela. É o tipo de defeito que só aparece para quem
// usa o app no idioma errado, que normalmente não é quem escreveu o código.
//
// Compara os caminhos FOLHA (`server.envVars.title`), não os objetos: é neles
// que a tradução mora, e é neles que a falta dói.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const raiz = join(dirname(fileURLToPath(import.meta.url)), "..");
const idiomas = ["en", "pt-BR"];

function folhas(valor, prefixo, saida) {
  if (valor && typeof valor === "object" && !Array.isArray(valor)) {
    for (const [k, v] of Object.entries(valor)) {
      folhas(v, prefixo ? `${prefixo}.${k}` : k, saida);
    }
  } else {
    saida.add(prefixo);
  }
  return saida;
}

/**
 * Chaves repetidas dentro de um mesmo objeto. O JSON.parse fica com a última
 * e não avisa: uma chave nova com o nome de uma que já existia mais abaixo
 * some sem erro (foi assim que o "Energia da GPU" do monitor virou "ENERGIA").
 * Uma varredura de tokens basta: strings (com escapes), chaves e dois-pontos.
 */
function duplicadas(texto) {
  const achadas = [];
  const pilha = []; // por objeto aberto: { caminho, chaves: Set }
  let i = 0;
  let ultimaString = null;
  let caminhoDaChave = null;
  while (i < texto.length) {
    const c = texto[i];
    if (c === '"') {
      let j = i + 1;
      let valor = "";
      while (texto[j] !== '"') {
        if (texto[j] === "\\") {
          valor += texto[j + 1];
          j += 2;
        } else {
          valor += texto[j];
          j += 1;
        }
      }
      ultimaString = valor;
      i = j + 1;
      continue;
    }
    if (c === ":" && pilha.length > 0 && ultimaString != null) {
      const topo = pilha[pilha.length - 1];
      const caminho = topo.caminho ? `${topo.caminho}.${ultimaString}` : ultimaString;
      if (topo.chaves.has(ultimaString)) achadas.push(caminho);
      topo.chaves.add(ultimaString);
      caminhoDaChave = caminho;
    } else if (c === "{") {
      pilha.push({ caminho: pilha.length === 0 ? "" : caminhoDaChave, chaves: new Set() });
    } else if (c === "}") {
      pilha.pop();
    }
    if (c !== " " && c !== "\n" && c !== "\r" && c !== "\t") ultimaString = c === ":" ? null : ultimaString;
    if (c === "," || c === "{" || c === "}") ultimaString = null;
    i += 1;
  }
  return achadas;
}

let repetidas = false;
for (const lng of idiomas) {
  const lista = duplicadas(readFileSync(join(raiz, "src", "i18n", `${lng}.json`), "utf8"));
  if (lista.length === 0) continue;
  repetidas = true;
  console.error(`\n${lng}.json tem chave(s) repetida(s) no mesmo objeto (vale só a última):`);
  for (const k of lista) console.error(`  ${k}`);
}
if (repetidas) process.exit(1);

const mapa = new Map(
  idiomas.map((lng) => [
    lng,
    folhas(
      JSON.parse(readFileSync(join(raiz, "src", "i18n", `${lng}.json`), "utf8")),
      "",
      new Set(),
    ),
  ]),
);

let falhou = false;
for (const a of idiomas) {
  for (const b of idiomas) {
    if (a === b) continue;
    const faltando = [...mapa.get(a)].filter((k) => !mapa.get(b).has(k)).sort();
    if (faltando.length === 0) continue;
    falhou = true;
    console.error(`\n${faltando.length} chave(s) em ${a}.json e ausente(s) em ${b}.json:`);
    for (const k of faltando) console.error(`  ${k}`);
  }
}

if (falhou) {
  console.error("\nToda chave nova precisa entrar nos DOIS arquivos.");
  process.exit(1);
}
console.log(`i18n: ${mapa.get(idiomas[0]).size} chaves, paridade entre ${idiomas.join(" e ")}.`);
