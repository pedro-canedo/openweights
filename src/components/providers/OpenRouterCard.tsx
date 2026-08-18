// OpenRouter: chave, saldo e escolha dos modelos que aparecem no chat.
//
// Interface nativa em vez de embutir o site: openrouter.ai responde
// `X-Frame-Options: SAMEORIGIN` e `frame-ancestors 'self'`, então iframe está
// fora de questão. O catálogo é público (não exige chave), então dá para ver
// modelos e preços antes de decidir criar conta.

import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  joinModelRef,
  openRouterKeyInfo,
  openRouterModels,
  providersConfigGet,
  providersConfigSet,
  type KeyInfo,
  type OpenRouterModel,
  type ProvidersConfig,
} from "../../lib/providers";
import { errorMessage } from "../../lib/serverSession";

const input =
  "rounded-lg border border-edge bg-panel2 px-3 py-2 text-sm outline-none placeholder:text-dim focus:border-accent";

/** Preço por milhão de tokens: a unidade que as pessoas comparam. */
function precoPorMilhao(porToken: number | null): string | null {
  if (porToken == null) return null;
  const v = porToken * 1_000_000;
  return v === 0 ? null : `$${v < 1 ? v.toFixed(3) : v.toFixed(2)}`;
}

export default function OpenRouterCard() {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<ProvidersConfig | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [info, setInfo] = useState<KeyInfo | null>(null);
  const [models, setModels] = useState<OpenRouterModel[] | null>(null);
  const [busca, setBusca] = useState("");
  const [soGratis, setSoGratis] = useState(false);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void providersConfigGet().then((c) => {
      setCfg(c);
      setApiKey(c.openRouter.apiKey);
    });
  }, []);

  // O catálogo é público: carrega mesmo sem chave.
  useEffect(() => {
    void openRouterModels()
      .then(setModels)
      .catch(() => setModels([]));
  }, []);

  const favoritos = useMemo(
    () => new Set(cfg?.openRouter.favorites ?? []),
    [cfg],
  );

  const gravar = useCallback(
    async (proximo: ProvidersConfig) => {
      setBusy(true);
      setError(null);
      try {
        await providersConfigSet(proximo);
        setCfg(proximo);
        setSaved(true);
        window.setTimeout(() => setSaved(false), 2000);
      } catch (e) {
        setError(errorMessage(e));
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  async function salvarChave() {
    if (!cfg) return;
    const chave = apiKey.trim();
    const proximo: ProvidersConfig = {
      ...cfg,
      openRouter: { ...cfg.openRouter, apiKey: chave, enabled: chave.length > 0 },
    };
    await gravar(proximo);
    if (!chave) {
      setInfo(null);
      return;
    }
    // Testar logo após salvar: uma chave inválida precisa aparecer agora, não
    // na primeira conversa.
    try {
      setInfo(await openRouterKeyInfo());
    } catch (e) {
      setInfo(null);
      setError(errorMessage(e));
    }
  }

  function alternarFavorito(id: string) {
    if (!cfg) return;
    const atuais = cfg.openRouter.favorites;
    const proximos = atuais.includes(id)
      ? atuais.filter((x) => x !== id)
      : [...atuais, id];
    void gravar({
      ...cfg,
      openRouter: { ...cfg.openRouter, favorites: proximos },
    });
  }

  const visiveis = useMemo(() => {
    if (!models) return [];
    const termo = busca.trim().toLowerCase();
    return models
      .filter((m) => (soGratis ? m.isFree : true))
      .filter(
        (m) =>
          !termo ||
          m.id.toLowerCase().includes(termo) ||
          m.name.toLowerCase().includes(termo),
      )
      .slice(0, 60);
  }, [models, busca, soGratis]);

  return (
    <div className="rounded-xl border border-edge bg-panel px-5 py-4">
      <div className="text-sm">{t("providers.openRouter.title")}</div>
      <div className="mt-1 text-[12px] text-dim">
        {t("providers.openRouter.subtitle")}
      </div>

      <div className="mt-3 flex flex-col gap-2 sm:flex-row">
        <input
          type="password"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          placeholder={t("providers.openRouter.apiKeyPlaceholder")}
          aria-label={t("providers.openRouter.apiKey")}
          className={`flex-1 ${input}`}
        />
        <button
          onClick={() => void salvarChave()}
          disabled={busy || !cfg}
          className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white disabled:opacity-50"
        >
          {busy ? t("common.loading") : t("common.save")}
        </button>
      </div>

      {saved && <p className="mt-2 text-[12px] text-ok">{t("common.saved")}</p>}
      {info && (
        <p className="mt-2 text-[12px] text-dim">
          {t("providers.openRouter.usage", {
            usage: info.usage.toFixed(2),
            limit:
              info.limit == null
                ? t("providers.openRouter.noLimit")
                : `$${info.limit.toFixed(2)}`,
          })}
        </p>
      )}
      {error && (
        <p className="mt-2 rounded-lg border border-bad/40 bg-bad/10 px-3 py-2 text-[12px] text-bad">
          {error}
        </p>
      )}

      {/* Catálogo. Só os favoritos vão para o seletor do chat: são centenas
          de modelos, e despejar todos ali tornaria o seletor inútil. */}
      <div className="mt-4 border-t border-edge pt-3">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
          <input
            value={busca}
            onChange={(e) => setBusca(e.target.value)}
            placeholder={t("providers.openRouter.searchPlaceholder")}
            aria-label={t("providers.openRouter.search")}
            className={`flex-1 ${input}`}
          />
          <label className="flex items-center gap-2 text-[12px] text-dim">
            <input
              type="checkbox"
              checked={soGratis}
              onChange={(e) => setSoGratis(e.target.checked)}
            />
            {t("providers.openRouter.onlyFree")}
          </label>
        </div>

        {models == null && (
          <div className="mt-3 space-y-2">
            {[0, 1, 2].map((i) => (
              <div key={i} className="h-9 animate-pulse rounded-lg bg-panel2" />
            ))}
          </div>
        )}

        {models != null && (
          <ul className="mt-3 max-h-80 space-y-1 overflow-y-auto">
            {visiveis.map((m) => {
              const fav = favoritos.has(m.id);
              const entrada = precoPorMilhao(m.promptPrice);
              const saida = precoPorMilhao(m.completionPrice);
              return (
                <li
                  key={m.id}
                  className="flex items-center justify-between gap-3 rounded-lg border border-edge bg-panel2 px-3 py-2"
                >
                  <div className="min-w-0">
                    <div className="truncate text-[13px]">{m.name}</div>
                    <div className="truncate text-[11px] text-dim">
                      {m.id}
                      {m.contextLength
                        ? ` · ${Math.round(m.contextLength / 1024)}k`
                        : ""}
                      {m.isFree
                        ? ` · ${t("providers.openRouter.free")}`
                        : entrada && saida
                          ? ` · ${entrada}/${saida} ${t("providers.openRouter.perMillion")}`
                          : ""}
                      {m.supportsTools
                        ? ` · ${t("providers.openRouter.tools")}`
                        : ""}
                    </div>
                  </div>
                  <button
                    onClick={() => alternarFavorito(m.id)}
                    disabled={busy}
                    title={joinModelRef("openrouter", m.id)}
                    className={`shrink-0 rounded-lg border px-3 py-1.5 text-[12px] disabled:opacity-50 ${
                      fav
                        ? "border-accent text-accent"
                        : "border-edge text-dim hover:text-ink"
                    }`}
                  >
                    {fav
                      ? t("providers.openRouter.pinned")
                      : t("providers.openRouter.pin")}
                  </button>
                </li>
              );
            })}
            {visiveis.length === 0 && (
              <li className="px-1 py-2 text-[12px] text-dim">
                {t("providers.openRouter.noneFound")}
              </li>
            )}
          </ul>
        )}
      </div>
    </div>
  );
}
