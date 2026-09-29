// O comando do llama-server como a pessoa o colaria num terminal (bash/zsh):
// cada argumento entre aspas simples quando precisa, e as variáveis de
// ambiente na frente. A chave de API já chega mascarada do backend.

const SEGURO = /^[A-Za-z0-9_@%+=:,./-]+$/;

/** Aspas simples do POSIX: `'` dentro do texto vira `'\''`. */
export function aspas(arg: string): string {
  if (arg === "") return "''";
  return SEGURO.test(arg) ? arg : `'${arg.replace(/'/g, `'\\''`)}'`;
}

/**
 * `env` vem como `NOME=valor`. O valor ganha aspas; o nome é o que estiver
 * antes do primeiro `=`.
 */
export function comandoShell(exe: string, args: readonly string[], env: readonly string[]): string {
  const vars = env.map((e) => {
    const i = e.indexOf("=");
    return i < 0 ? aspas(e) : `${e.slice(0, i)}=${aspas(e.slice(i + 1))}`;
  });
  const corpo = [aspas(exe), ...args.map(aspas)].join(" ");
  return [...vars.map((v) => `${v} \\`), corpo].join("\n");
}
