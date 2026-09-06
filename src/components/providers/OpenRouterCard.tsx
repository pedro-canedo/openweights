// OpenRouter: chave, saldo e escolha dos modelos que aparecem no chat.
//
// Interface nativa em vez de embutir o site: openrouter.ai responde
// `X-Frame-Options: SAMEORIGIN` e `frame-ancestors 'self'`, então iframe está
// fora de questão. O catálogo é público (não exige chave), então dá para ver
// modelos e preços antes de decidir criar conta.
//
// O card fazia duas coisas muito diferentes em sequência — guardar uma
// credencial e escolher entre 400 modelos — com o mesmo peso visual, e a
// segunda enterrava a primeira. Agora são dois blocos: a chave, com o estado
// dela à vista (uma senha preenchida é idêntica a uma vazia), e o catálogo,
// que ganhou o que faltava para ser usável: quantos modelos existem, quantos
// estão fixados e um jeito de rever só os fixados — que são os únicos que
// aparecem no seletor do chat.

import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
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
import { Card, StatusDot } from "../ui/Shell";

const input =
  "rounded-lg border border-edge bg-panel2 px-3 py-2 text-sm outline-none placeholder:text-dim focus:border-accent";

/** Teto da lista. Renderizar 400 linhas trava a rolagem e não ajuda ninguém
 *  — quem procura algo específico digita. */
const MAX_VISIVEIS = 60;

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
  const [soFixados, setSoFixados] = useState(false);
  const [editandoChave, setEditandoChave] = useState(false);
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

  const gravar = useCallback(async (proximo: ProvidersConfig) => {
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
  }, []);

  async function salvarChave() {
    if (!cfg) return;
    const chave = apiKey.trim();
    const proximo: ProvidersConfig = {
      ...cfg,
      openRouter: { ...cfg.openRouter, apiKey: chave, enabled: chave.length > 0 },
    };
    await gravar(proximo);
    setEditandoChave(false);
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

  const filtrados = useMemo(() => {
    if (!models) return [];
    const termo = busca.trim().toLowerCase();
    return models
      .filter((m) => (soGratis ? m.isFree : true))
      .filter((m) => (soFixados ? favoritos.has(m.id) : true))
      .filter(
        (m) =>
          !termo ||
          m.id.toLowerCase().includes(termo) ||
          m.name.toLowerCase().includes(termo),
      );
  }, [models, busca, soGratis, soFixados, favoritos]);

  const visiveis = filtrados.slice(0, MAX_VISIVEIS);
  const temChave = !!cfg?.openRouter.apiKey;

  return (
    <>
      {/* ------------------------------------------------------- a chave */}
      <Card
        title={t("providers.openRouter.title")}
        hint={t("providers.openRouter.subtitle")}
        action={
          temChave && !editandoChave ? (
            <button
              type="button"
              onClick={() => setEditandoChave(true)}
              className="rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:border-accent hover:text-ink"
            >
              {t("settings.hfTokenChange")}
            </button>
          ) : undefined
        }
      >
        <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2">
          <StatusDot tone={temChave ? "ok" : "off"} />
          <span className="text-sm">
            {temChave
              ? t("providers.openRouter.keySet")
              : t("providers.openRouter.keyNone")}
          </span>
          {info && (
            <span className="text-[12px] text-dim">
              {t("providers.openRouter.usage", {
                usage: info.usage.toFixed(2),
                limit:
                  info.limit == null
                    ? t("providers.openRouter.noLimit")
                    : `$${info.limit.toFixed(2)}`,
              })}
            </span>
          )}
          {saved && <span className="text-[12px] text-ok">{t("common.saved")}</span>}
        </div>

        {(!temChave || editandoChave) && (
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
        )}

        {error && (
          <p className="mt-2 rounded-lg border border-bad/40 bg-bad/10 px-3 py-2 text-[12px] text-bad">
            {error}
          </p>
        )}
      </Card>

      {/* ----------------------------------------------------- o catálogo */}
      <Card
        title={t("providers.openRouter.catalog")}
        hint={t("providers.openRouter.catalogHint")}
        action={
          <span className="text-[12px] text-dim">
            {t("providers.openRouter.pinnedCount", { n: favoritos.size })}
          </span>
        }
      >
        <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center">
          <input
            value={busca}
            onChange={(e) => setBusca(e.target.value)}
            placeholder={t("providers.openRouter.searchPlaceholder")}
            aria-label={t("providers.openRouter.search")}
            className={`flex-1 ${input}`}
          />
          <div className="flex shrink-0 items-center gap-3">
            <Filtro
              ativo={soGratis}
              onClick={() => setSoGratis((v) => !v)}
              label={t("providers.openRouter.onlyFree")}
            />
            <Filtro
              ativo={soFixados}
              onClick={() => setSoFixados((v) => !v)}
              label={t("providers.openRouter.onlyPinned")}
            />
          </div>
        </div>

        {models == null && (
          <div className="mt-3 space-y-2">
            {[0, 1, 2].map((i) => (
              <div key={i} className="h-9 animate-pulse rounded-lg bg-panel2" />
            ))}
          </div>
        )}

        {models != null && (
          <>
            <ul className="mt-3 max-h-96 space-y-1 overflow-y-auto pr-1">
              {visiveis.map((m) => {
                const fav = favoritos.has(m.id);
                const entrada = precoPorMilhao(m.promptPrice);
                const saida = precoPorMilhao(m.completionPrice);
                return (
                  <li
                    key={m.id}
                    className={`flex items-center justify-between gap-3 rounded-lg border bg-panel2 px-3 py-2 ${
                      fav ? "border-accent/40" : "border-edge"
                    }`}
                  >
                    <div className="min-w-0">
                      <div className="truncate text-[13px]">{m.name}</div>
                      <div className="mt-0.5 flex flex-wrap items-center gap-1.5 text-[11px] text-dim">
                        <span className="truncate font-mono">{m.id}</span>
                        {m.contextLength != null && (
                          <Etiqueta>
                            {Math.round(m.contextLength / 1024)}k
                          </Etiqueta>
                        )}
                        {m.isFree ? (
                          <Etiqueta tone="ok">
                            {t("providers.openRouter.free")}
                          </Etiqueta>
                        ) : entrada && saida ? (
                          <Etiqueta>
                            {entrada}/{saida}{" "}
                            {t("providers.openRouter.perMillion")}
                          </Etiqueta>
                        ) : null}
                        {m.supportsTools && (
                          <Etiqueta>{t("providers.openRouter.tools")}</Etiqueta>
                        )}
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
            {/* Dizer que a lista foi cortada é o que evita a conclusão de que
                o modelo procurado não existe no OpenRouter. */}
            <p className="mt-2 text-[11px] text-dim">
              {filtrados.length > visiveis.length
                ? t("providers.openRouter.showing", {
                    n: visiveis.length,
                    total: filtrados.length,
                  })
                : t("providers.openRouter.total", { n: filtrados.length })}
            </p>
          </>
        )}
      </Card>
    </>
  );
}

function Filtro({
  ativo,
  onClick,
  label,
}: {
  ativo: boolean;
  onClick: () => void;
  label: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={ativo}
      className={`rounded-lg border px-2.5 py-1.5 text-[12px] transition-colors ${
        ativo
          ? "border-accent text-accent"
          : "border-edge text-dim hover:text-ink"
      }`}
    >
      {label}
    </button>
  );
}

function Etiqueta({
  children,
  tone,
}: {
  children: ReactNode;
  tone?: "ok";
}) {
  return (
    <span
      className={`rounded-full px-1.5 py-0.5 text-[10px] ${
        tone === "ok" ? "bg-ok/10 text-ok" : "bg-panel text-dim"
      }`}
    >
      {children}
    </span>
  );
}
