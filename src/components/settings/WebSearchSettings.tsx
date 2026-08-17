// Configuração da busca na web do agente.
//
// Sem chave, a busca usa o DuckDuckGo por raspagem: grátis, qualidade menor
// e sujeito a bloqueio. Quem quiser resultado melhor cola uma chave do Brave
// ou da Tavily aqui — antes desta tela isso exigia `settings_set` na mão e
// reiniciar o app. Salvar chama `web_config_set`, que valida, grava e refaz
// o catálogo de ferramentas: a mudança vale na hora.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { webConfigGet, webConfigSet } from "../../lib/agent/agentApi";

/**
 * Valores que o serde do Rust aceita em `search.provider` (camelCase — é
 * `duckDuckGo`, não `duckduckgo`; o backend recusa o valor errado).
 */
const PROVIDERS = ["auto", "duckDuckGo", "brave", "tavily"] as const;
type Provider = (typeof PROVIDERS)[number];

/** Rótulo de cada provedor no select (chave i18n relativa). */
const PROVIDER_LABEL: Record<Provider, string> = {
  auto: "providerAuto",
  duckDuckGo: "providerDuckDuckGo",
  brave: "providerBrave",
  tavily: "providerTavily",
};

export default function WebSearchSettings() {
  const { t } = useTranslation();
  const [provider, setProvider] = useState<Provider>("auto");
  const [apiKey, setApiKey] = useState("");
  // Configuração completa como está gravada: salvar preserva os campos que
  // esta tela não edita (timeoutSecs etc.) em vez de apagá-los.
  const [stored, setStored] = useState<Record<string, unknown>>({});
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void webConfigGet().then((raw) => {
      if (!raw) return;
      // Parse defensivo: o setting pode ter sido escrito à mão. JSON
      // estragado deixa a tela nos padrões — igual ao backend.
      try {
        const cfg = JSON.parse(raw) as unknown;
        if (!cfg || typeof cfg !== "object") return;
        setStored(cfg as Record<string, unknown>);
        const search = (cfg as Record<string, unknown>).search;
        if (!search || typeof search !== "object") return;
        const { provider: p, apiKey: key } = search as Record<string, unknown>;
        if (typeof p === "string" && (PROVIDERS as readonly string[]).includes(p)) {
          setProvider(p as Provider);
        }
        if (typeof key === "string") setApiKey(key);
      } catch (e) {
        console.warn("web.config ilegível; a tela começa nos padrões:", e);
      }
    });
  }, []);

  async function save() {
    if (busy) return;
    setBusy(true);
    setError(null);
    const key = apiKey.trim();
    const previousSearch =
      stored.search && typeof stored.search === "object"
        ? (stored.search as Record<string, unknown>)
        : {};
    const next = {
      ...stored,
      // Campos parciais são válidos (o `WebConfig` tem padrão em tudo);
      // chave vazia vira `null` para não gravar uma string em branco.
      search: { ...previousSearch, provider, apiKey: key === "" ? null : key },
    };
    try {
      await webConfigSet(JSON.stringify(next));
      setStored(next);
      setApiKey(key);
      setSaved(true);
      setTimeout(() => setSaved(false), 2500);
    } catch (e) {
      setError(t("settings.webSearch.saveFailed", { error: String(e) }));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="rounded-xl border border-edge bg-panel px-5 py-4">
      <div className="text-sm">{t("settings.webSearch.title")}</div>
      <div className="mt-1 text-[12px] text-dim">
        {t("settings.webSearch.subtitle")}
      </div>

      <div className="mt-3 flex flex-col gap-2 sm:flex-row">
        <select
          value={provider}
          onChange={(e) => setProvider(e.target.value as Provider)}
          aria-label={t("settings.webSearch.provider")}
          className="rounded-lg border border-edge bg-panel2 px-3 py-2 text-sm outline-none focus:border-accent"
        >
          {PROVIDERS.map((p) => (
            <option key={p} value={p}>
              {t(`settings.webSearch.${PROVIDER_LABEL[p]}`)}
            </option>
          ))}
        </select>
        <input
          type="password"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          placeholder={t("settings.webSearch.apiKeyPlaceholder")}
          aria-label={t("settings.webSearch.apiKey")}
          className="flex-1 rounded-lg border border-edge bg-panel2 px-3 py-2 text-sm outline-none placeholder:text-dim focus:border-accent"
        />
        <button
          onClick={() => void save()}
          disabled={busy}
          className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white disabled:opacity-50"
        >
          {busy ? t("common.loading") : t("common.save")}
        </button>
      </div>

      {saved && (
        <p className="mt-2 text-[12px] text-ok">
          {t("settings.webSearch.saved")}
        </p>
      )}
      {error && (
        <p className="mt-2 rounded-lg border border-bad/40 bg-bad/10 px-3 py-2 text-[12px] text-bad">
          {error}
        </p>
      )}

      <p className="mt-3 text-[11px] text-dim">
        {t("settings.webSearch.applyHint")}
      </p>
    </div>
  );
}
