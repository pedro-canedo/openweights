// Foto do autor/organização no Hub, com as iniciais coloridas por baixo.
//
// A foto é `huggingface.co/{autor}.png` — um caminho previsível, sem uma
// chamada de API por autor. Ela entra POR CIMA das iniciais em vez de
// substituí-las: enquanto carrega (ou quando o autor não tem foto) o que se
// vê é um bloco com as letras, nunca um buraco cinza que pula quando a
// imagem chega.

import { useState } from "react";

const TONS = [
  "bg-sky-500/20 text-sky-300",
  "bg-violet-500/20 text-violet-300",
  "bg-emerald-500/20 text-emerald-300",
  "bg-amber-500/20 text-amber-300",
  "bg-rose-500/20 text-rose-300",
  "bg-cyan-500/20 text-cyan-300",
  "bg-orange-500/20 text-orange-300",
  "bg-fuchsia-500/20 text-fuchsia-300",
];

/** Cor estável por autor: o mesmo nome cai sempre no mesmo tom. */
function tom(nome: string): string {
  let h = 0;
  for (let i = 0; i < nome.length; i++) h = (h * 31 + nome.charCodeAt(i)) >>> 0;
  return TONS[h % TONS.length];
}

function iniciais(autor: string): string {
  const limpo = autor.replace(/[-_.]+/g, " ").trim();
  const partes = limpo.split(/\s+/).filter(Boolean);
  if (partes.length >= 2) return (partes[0][0] + partes[1][0]).toUpperCase();
  return (limpo.slice(0, 2) || "?").toUpperCase();
}

export default function AuthorAvatar({
  author,
  size = 40,
  className = "rounded-xl",
}: {
  author: string;
  size?: number;
  className?: string;
}) {
  const [falhou, setFalhou] = useState(!author);
  const src = author
    ? `https://huggingface.co/${encodeURIComponent(author)}.png`
    : "";

  return (
    <span
      className={`relative inline-flex shrink-0 items-center justify-center overflow-hidden ${className} ${tom(author || "?")}`}
      style={{ width: size, height: size }}
      aria-hidden
    >
      <span
        className="font-semibold tracking-wide"
        style={{ fontSize: Math.max(10, Math.round(size * 0.28)) }}
      >
        {iniciais(author || "?")}
      </span>
      {!falhou && (
        <img
          src={src}
          alt=""
          width={size}
          height={size}
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          className="absolute inset-0 h-full w-full object-cover"
          onError={() => setFalhou(true)}
        />
      )}
    </span>
  );
}
