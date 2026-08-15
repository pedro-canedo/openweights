// Tela Descobrir: busca de modelos GGUF no Hugging Face com debounce,
// ordenação, cards de resultado e drawer de quantizações com veredito
// de compatibilidade por hardware.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ModelSummary } from "../lib/types";
import { searchModels, type SearchSort } from "../lib/api";
import ModelCard from "../components/discover/ModelCard";
import QuantDrawer from "../components/discover/QuantDrawer";

const SORTS: SearchSort[] = ["trending", "downloads", "likes", "updated"];

const SORT_KEYS: Record<SearchSort, string> = {
  trending: "discover.sortTrending",
  downloads: "discover.sortDownloads",
  likes: "discover.sortLikes",
  updated: "discover.sortUpdated",
};

export default function Discover() {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [sort, setSort] = useState<SearchSort>("trending");
  const [results, setResults] = useState<ModelSummary[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [reload, setReload] = useState(0);
  const [selected, setSelected] = useState<ModelSummary | null>(null);
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
        if (seq.current === my) setResults(r);
      })
      .catch((err) => {
        console.error(err);
        if (seq.current === my) {
          setFailed(true);
          setResults([]);
        }
      });
  }, [debounced, sort, reload]);

  return (
    <div className="mx-auto max-w-4xl px-8 py-8">
      <h1 className="text-xl font-semibold">{t("discover.title")}</h1>
      <p className="mt-1 text-sm text-dim">{t("discover.subtitle")}</p>

      <div className="mt-6 flex gap-2">
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("discover.searchPlaceholder")}
          className="min-w-0 flex-1 rounded-xl border border-edge bg-panel px-4 py-3 text-sm outline-none placeholder:text-dim focus:border-accent"
        />
        <select
          value={sort}
          onChange={(e) => setSort(e.target.value as SearchSort)}
          className="shrink-0 rounded-xl border border-edge bg-panel px-3 py-3 text-sm text-ink outline-none focus:border-accent"
        >
          {SORTS.map((s) => (
            <option key={s} value={s}>
              {t(SORT_KEYS[s])}
            </option>
          ))}
        </select>
      </div>

      <div className="mt-6">
        {failed ? (
          <div className="rounded-xl border border-dashed border-edge p-10 text-center">
            <p className="text-sm text-dim">{t("common.error")}</p>
            <button
              onClick={() => setReload((n) => n + 1)}
              className="mt-3 rounded-lg border border-edge bg-panel px-4 py-2 text-sm font-medium text-ink transition-colors hover:border-accent"
            >
              {t("common.retry")}
            </button>
          </div>
        ) : results == null ? (
          <div className="flex flex-col gap-3">
            {Array.from({ length: 4 }).map((_, i) => (
              <div
                key={i}
                className="animate-pulse rounded-xl border border-edge bg-panel p-5"
              >
                <div className="h-4 w-2/5 rounded bg-panel2" />
                <div className="mt-3 h-3 w-3/5 rounded bg-panel2" />
              </div>
            ))}
          </div>
        ) : results.length === 0 ? (
          <div className="rounded-xl border border-dashed border-edge p-10 text-center text-sm text-dim">
            {t("discover.empty")}
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            {results.map((m) => (
              <ModelCard key={m.id} model={m} onClick={() => setSelected(m)} />
            ))}
          </div>
        )}
      </div>

      {selected && (
        <QuantDrawer model={selected} onClose={() => setSelected(null)} />
      )}
    </div>
  );
}
