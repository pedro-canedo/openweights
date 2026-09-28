// Os pacotes npm do Tauri precisam do mesmo major.minor dos crates Rust.
//
// O `tauri build` recusa o empacotamento quando não batem ("Found version
// mismatched Tauri packages"), mas só a Release roda `tauri build`: a tag
// v0.25.0 caiu nos três sistemas porque instalar
// `@tauri-apps/plugin-notification` puxou o `@tauri-apps/api` para 2.12
// enquanto o `tauri` segue em 2.11. Esta checagem roda no `npm run build`,
// então a CI de todo push pega o que antes só aparecia depois da tag.
//
//   @tauri-apps/api           ↔ tauri
//   @tauri-apps/plugin-<nome> ↔ tauri-plugin-<nome>

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const raiz = join(dirname(fileURLToPath(import.meta.url)), "..");
const ler = (...p) => readFileSync(join(raiz, ...p), "utf8");

const pacotes = JSON.parse(ler("package-lock.json")).packages;
const crates = new Map();
for (const bloco of ler("src-tauri", "Cargo.lock").split("[[package]]")) {
  const nome = /^name = "([^"]+)"/m.exec(bloco)?.[1];
  const versao = /^version = "([^"]+)"/m.exec(bloco)?.[1];
  if (nome && versao) crates.set(nome, versao);
}

const menor = (v) => v.split(".").slice(0, 2).join(".");
const erros = [];
let conferidos = 0;
for (const [caminho, { version }] of Object.entries(pacotes)) {
  const m = /^node_modules\/@tauri-apps\/(api|plugin-[\w-]+)$/.exec(caminho);
  if (!m) continue;
  const crate = m[1] === "api" ? "tauri" : `tauri-${m[1]}`;
  const doCrate = crates.get(crate);
  if (!doCrate) {
    erros.push(`@tauri-apps/${m[1]} ${version} não tem o crate ${crate} no Cargo.lock`);
    continue;
  }
  conferidos++;
  if (menor(version) !== menor(doCrate)) {
    erros.push(
      `@tauri-apps/${m[1]} ${version} × ${crate} ${doCrate}: fixe o npm em ~${menor(doCrate)} (npm install @tauri-apps/${m[1]}@~${menor(doCrate)})`,
    );
  }
}

if (erros.length) {
  console.error("tauri-versions: pacotes npm e crates do Tauri desalinhados");
  for (const e of erros) console.error(`  ✗ ${e}`);
  process.exit(1);
}
console.log(`tauri-versions: ${conferidos} pacotes npm alinhados com os crates`);
