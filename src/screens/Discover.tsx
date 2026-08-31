// Tela Descobrir: busca de modelos GGUF no Hub, em duas colunas.
//
// A lista escolhe, o painel decide. Antes eram cards largos empilhados: cada
// modelo custava um terço da tela, comparar dois exigia rolar, e a única
// forma de ver as quantizações era abrir uma gaveta que cobria tudo — com o
// resultado de que, ao fechar, o lugar na lista se perdia. Com a lista
// estreita à esquerda e o detalhe fixo à direita, trocar de modelo é um
// clique e nada some do caminho.
//
// A primeira linha do resultado já vem selecionada: uma tela que abre com
// metade vazia esperando um clique desperdiça o que ela já sabe mostrar.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ModelSummary } from "../lib/types";
import { searchModels, type SearchSort } from "../lib/api";
import ModelListItem from "../components/discover/ModelListItem";
import ModelDetail from "../components/discover/ModelDetail";

const SORTS: SearchSort[] = ["trending", "downloads", "likes", "updated"];

const SORT_KEYS: Record<SearchSort, string> = {
  trending: "discover.sortTrending",
  downloads: "discover.sortDownloads",
  likes: "discover.sortLikes",
  updated: "discover.sortUpdated",
};

/** Tira o nome do formato da busca, deixando só o nome do modelo. */
function semFormato(q: string): string {
  return q
    .replace(/\b(mlx|ollama|safetensors|gptq|awq)\b/gi, "")
    .replace(/\s{2,}/g, " ")
    .trim();
}

export default function Discover() {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  // O app roda arquivos GGUF pelo llama.cpp: MLX é o formato da Apple e não
  // roda aqui, e "tag" é vocabulário do Ollama.
  const outroFormato = /\b(mlx|ollama|safetensors|gptq|awq)\b/i.test(query);
  const [debounced, setDebounced] = useState("");
  const [sort, setSort] = useState<SearchSort>("trending");
  const [results, setResults] = useState<ModelSummary[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [reload, setReload] = useState(0);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const seq = useRef(0);

  // Debounce de 400 ms na digitação.
  useEffect(() => {
    const id = setTimeout(() => setDebounced(query), 400);
    return () => clearTimeout(id);
  }, [query]);

  // Busca (vazia = "em alta"); guarda contra respostas fora de ordem.
  useEffect(() => {
    const my = ++seq.current;
    setResults(null);
    setFailed(false);
    searchModels(debounced, sort)
      .then((r) => {
        if (seq.current !== my) return;
        setResults(r);
        // O selecionado só muda quando sai do resultado: quem trocou a
        // ordenação continua olhando o mesmo modelo, se ele ainda está aí.
        setSelectedId((atual) =>
          atual && r.some((m) => m.id === atual) ? atual : (r[0]?.id ?? null),
        );
      })
      .catch((err) => {
        console.error(err);
        if (seq.current !== my) return;
        setFailed(true);
        setResults([]);
        setSelectedId(null);
      });
  }, [debounced, sort, reload]);

  const selected = results?.find((m) => m.id === selectedId) ?? null;

  return (
    <div className="flex h-full min-h-0">
      {/* --------------------------------------------------- a lista */}
      <div className="flex w-[380px] shrink-0 flex-col border-r border-edge">
        <div className="shrink-0 px-4 pt-5 pb-3">
          <h1 className="text-base font-semibold">{t("discover.title")}</h1>
          <p className="mt-0.5 text-[12px] leading-relaxed text-dim">
            {t("discover.subtitle")}
          </p>

          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("discover.searchPlaceholder")}
            className="mt-3 w-full rounded-xl border border-edge bg-panel px-3 py-2 text-[13px] outline-none placeholder:text-dim focus:border-accent"
          />

          <div className="mt-2 flex items-center gap-2">
            <span className="text-[11px] text-dim">{t("discover.sortBy")}</span>
            <select
              value={sort}
              onChange={(e) => setSort(e.target.value as SearchSort)}
              className="min-w-0 flex-1 rounded-lg border border-edge bg-panel px-2 py-1.5 text-[12px] text-ink outline-none focus:border-accent"
            >
              {SORTS.map((s) => (
                <option key={s} value={s}>
                  {t(SORT_KEYS[s])}
                </option>
              ))}
            </select>
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-4">
          {failed ? (
            <div className="mx-2 rounded-xl border border-dashed border-edge p-6 text-center">
              <p className="text-[13px] text-dim">{t("common.error")}</p>
              <button
                onClick={() => setReload((n) => n + 1)}
                className="mt-3 rounded-lg border border-edge bg-panel px-3 py-1.5 text-[12px] font-medium text-ink transition-colors hover:border-accent"
              >
                {t("common.retry")}
              </button>
            </div>
          ) : results == null ? (
            <div className="flex flex-col gap-1">
              {Array.from({ length: 8 }).map((_, i) => (
                <div
                  key={i}
                  className="flex animate-pulse items-center gap-3 rounded-xl px-3 py-2.5"
                >
                  <div className="h-[34px] w-[34px] shrink-0 rounded-lg bg-panel2" />
                  <div className="min-w-0 flex-1">
                    <div className="h-3 w-3/5 rounded bg-panel2" />
                    <div className="mt-2 h-2.5 w-2/5 rounded bg-panel2" />
                  </div>
                </div>
              ))}
            </div>
          ) : results.length === 0 ? (
            <div className="mx-2 rounded-xl border border-dashed border-edge p-6 text-center text-[13px] text-dim">
              {/* Procurar por "mlx" ou "ollama" não é erro de digitação: é
                  alguém trazendo o vocabulário de outra ferramenta. Um beco
                  sem saída aqui vira uma explicação e um caminho. */}
              {outroFormato ? (
                <div className="flex flex-col items-center gap-2">
                  <p className="leading-relaxed">{t("discover.otherFormat")}</p>
                  <button
                    type="button"
                    onClick={() => setQuery(semFormato(query))}
                    className="rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-white transition-opacity hover:opacity-90"
                  >
                    {t("discover.findGguf")}
                  </button>
                </div>
              ) : (
                t("discover.empty")
              )}
            </div>
          ) : (
            <div className="flex flex-col gap-1">
              {results.map((m) => (
                <ModelListItem
                  key={m.id}
                  model={m}
                  selected={m.id === selectedId}
                  onSelect={() => setSelectedId(m.id)}
                />
              ))}
            </div>
          )}
        </div>
      </div>

      {/* -------------------------------------------------- o detalhe */}
      <div className="min-w-0 flex-1">
        {selected ? (
          <ModelDetail key={selected.id} model={selected} />
        ) : (
          <div className="flex h-full items-center justify-center px-6">
            <p className="max-w-xs text-center text-[13px] leading-relaxed text-dim">
              {t("discover.pickOne")}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
