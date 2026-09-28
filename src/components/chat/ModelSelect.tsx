// Seletor combinado de modelo + esforço, no estilo do picker do composer:
// gatilho "Nome + esforço", lista de modelos e esforço / mais modelos no rodapé.

import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useDismiss } from "../ui/camada";
import { routerModels } from "../../lib/flags";
import { useModo } from "../../lib/mode";
import { normalizar } from "../../lib/comandos";
import { splitModelRef } from "../../lib/providers";
import {
  EFFORT_MAX_TOKENS,
  type ChatParams,
  type EffortLevel,
} from "../../lib/types";

const EFFORTS: EffortLevel[] = ["low", "medium", "high", "extra", "max"];
const PRIMARY_LIMIT = 4;

function shortModel(name: string): string {
  // O prefixo de provedor é ruído no rótulo: quem escolhe já vê a etiqueta
  // ao lado, e a linha de baixo mostra a referência inteira.
  const { model } = splitModelRef(name);
  return model
    .replace(/\.gguf$/i, "")
    .replace(/-UD-.*$/i, "")
    .replace(/-Q\d.*$/i, "");
}

/** Etiqueta do provedor, ausente no local (que é o caso comum). */
function Origem({ modelRef }: { modelRef: string }) {
  const { provider } = splitModelRef(modelRef);
  if (provider === "local") return null;
  return (
    <span className="shrink-0 rounded-md bg-panel2 px-1.5 py-0.5 text-[10px] text-dim">
      {provider}
    </span>
  );
}

function Check() {
  return (
    <svg
      className="h-4 w-4 shrink-0 text-sky-400 light:text-sky-700"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.4"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      <path d="M20 6L9 17l-5-5" />
    </svg>
  );
}

function Chevron({ dir = "down" }: { dir?: "down" | "right" }) {
  return (
    <svg
      className="h-3 w-3 shrink-0 text-dim"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
    >
      {dir === "down" ? (
        <path d="M6 9l6 6 6-6" />
      ) : (
        <path d="M9 6l6 6-6 6" />
      )}
    </svg>
  );
}


function Submenu({ children }: { children: ReactNode }) {
  return (
    <div
      data-overlay=""
      className="absolute right-full bottom-0 z-40 mr-1 min-w-48 rounded-xl border border-edge bg-panel py-1.5 shadow-[0_12px_40px_rgba(0,0,0,0.45)]"
    >
      {children}
    </div>
  );
}

function splitModels(options: string[], selected: string) {
  if (options.length <= PRIMARY_LIMIT) {
    return { primary: options, extra: [] as string[] };
  }
  const rest = options.filter((m) => m !== selected);
  const primary = [selected, ...rest].filter(Boolean).slice(0, PRIMARY_LIMIT);
  const extra = options.filter((m) => !primary.includes(m));
  return { primary, extra };
}

export default function ModelSelect({
  models,
  value,
  onChange,
  params,
  onParamsChange,
  disabled = false,
}: {
  models: string[];
  value: string;
  onChange: (model: string) => void;
  params: ChatParams;
  onParamsChange: (p: ChatParams) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  // O esforço de raciocínio é ajuste fino: no Simples o seletor some e vale o
  // da conversa (Alto, se ninguém mexeu).
  const avancado = useModo() === "avancado";
  const [open, setOpen] = useState(false);
  const [panel, setPanel] = useState<"effort" | "more" | null>(null);
  const ref = useDismiss(open, () => {
    setOpen(false);
    setPanel(null);
  });

  const [busca, setBusca] = useState("");
  // Os modelos que o Router tem na memória agora: trocar para um deles é
  // instantâneo; para os outros, a primeira resposta espera carregar.
  const [carregados, setCarregados] = useState<Set<string>>(new Set());
  useEffect(() => {
    if (!open) return;
    setBusca("");
    routerModels().then(
      (ms) => setCarregados(new Set(ms.filter((m) => m.state === "loaded").map((m) => m.id))),
      () => setCarregados(new Set()),
    );
  }, [open]);

  const options =
    value && !models.includes(value) ? [value, ...models] : models;
  const empty = options.length === 0;
  const termos = normalizar(busca).split(/\s+/).filter(Boolean);
  const { primary, extra } =
    termos.length > 0
      ? {
          primary: options.filter((m) => termos.every((t) => normalizar(m).includes(t))),
          extra: [] as string[],
        }
      : splitModels(options, value);

  const pickModel = (model: string) => {
    onChange(model);
    setOpen(false);
    setPanel(null);
  };

  const pickEffort = (effort: EffortLevel) => {
    onParamsChange({
      ...params,
      effort,
      maxTokens: EFFORT_MAX_TOKENS[effort],
    });
    setPanel(null);
  };

  return (
    <div ref={ref} className="relative shrink-0">
      <button
        type="button"
        disabled={disabled || empty}
        onClick={() => {
          setOpen((v) => !v);
          setPanel(null);
        }}
        aria-expanded={open}
        aria-haspopup="true"
        title={
          empty
            ? t("chat.modelSelect")
            : avancado
              ? `${value} · ${t("chat.effort.label")}: ${t(`chat.effort.${params.effort}`)}`
              : value
        }
        className={`flex max-w-56 items-center gap-1.5 rounded-full px-2 py-1 text-xs transition-colors disabled:opacity-40 hover:bg-panel ${
          open ? "bg-panel text-ink" : "text-ink"
        }`}
      >
        {/* O nome do modelo é o que a pessoa procura aqui; o esforço é
            informação de canto. */}
        <span className="truncate">
          {empty ? t("chat.modelSelect") : shortModel(value)}
        </span>
        {!empty && avancado && (
          <span className="shrink-0 text-dim">
            {t(`chat.effort.${params.effort}`)}
          </span>
        )}
        <Chevron />
      </button>

      {open && (
        <div
          data-overlay=""
          className="absolute right-0 bottom-full z-30 mb-2 w-80 rounded-xl border border-edge bg-panel py-1.5 shadow-[0_12px_40px_rgba(0,0,0,0.45)]"
        >
          {options.length > PRIMARY_LIMIT && (
            <div className="px-2 pb-1.5">
              <input
                type="search"
                aria-label={t("chat.searchModel")}
                placeholder={t("chat.searchModel")}
                value={busca}
                autoFocus
                onChange={(e) => {
                  setBusca(e.target.value);
                  setPanel(null);
                }}
                className="w-full rounded-lg border border-edge-strong bg-panel2 px-2.5 py-1.5 text-[12px] text-ink placeholder:text-dim outline-none focus:border-accent-ink"
              />
            </div>
          )}
          {termos.length > 0 && primary.length === 0 && (
            <p className="px-3 py-2 text-[12px] text-dim">{t("chat.noModelMatch")}</p>
          )}
          {primary.map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => pickModel(m)}
              className="flex w-full items-start gap-2 px-3 py-2.5 text-left hover:bg-panel2"
            >
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-1.5">
                  {carregados.has(m) && (
                    <span
                      role="img"
                      aria-label={t("chat.modelLoaded")}
                      title={t("chat.modelLoaded")}
                      className="h-1.5 w-1.5 shrink-0 rounded-full bg-ok"
                    />
                  )}
                  <span className="min-w-0 truncate text-[13px] text-ink">
                    {shortModel(m)}
                  </span>
                  <Origem modelRef={m} />
                </span>
                <span className="mt-0.5 block truncate text-[11px] text-dim">
                  {m}
                </span>
              </span>
              {value === m && <Check />}
            </button>
          ))}

          {avancado && (
            <>
              <div className="my-1.5 border-t border-edge" />

              <div className="relative" onMouseEnter={() => setPanel("effort")}>
                <button
                  type="button"
                  onClick={() =>
                    setPanel((cur) => (cur === "effort" ? null : "effort"))
                  }
                  className="flex w-full items-center gap-2 px-3 py-2.5 text-left text-[13px] hover:bg-panel2"
                >
                  <span className="flex-1 text-ink">{t("chat.effort.label")}</span>
                  <span className="text-dim">
                    {t(`chat.effort.${params.effort}`)}
                  </span>
                  <Chevron dir="right" />
                </button>
                {panel === "effort" && (
                  <Submenu>
                    <p className="px-3 pt-1.5 pb-2 text-[11px] leading-relaxed text-dim">
                      {t("chat.effort.hint")}
                    </p>
                    {EFFORTS.map((level) => (
                      <button
                        key={level}
                        type="button"
                        onClick={() => pickEffort(level)}
                        className="flex w-full items-center gap-2 px-3 py-2.5 text-left text-[13px] text-ink hover:bg-panel2"
                      >
                        <span>{t(`chat.effort.${level}`)}</span>
                        {level === "high" && (
                          <span className="rounded-md bg-panel2 px-1.5 py-0.5 text-[10px] text-dim">
                            {t("chat.effort.default")}
                          </span>
                        )}
                        {level === "max" && (
                          <span
                            title={t("chat.effort.maxHint")}
                            className="flex h-4 w-4 items-center justify-center rounded-full border border-dim/50 text-[9px] text-dim"
                          >
                            i
                          </span>
                        )}
                        <span className="ml-auto">
                          {params.effort === level ? <Check /> : null}
                        </span>
                      </button>
                    ))}
                  </Submenu>
                )}
              </div>
            </>
          )}

          {extra.length > 0 && (
            <div className="relative" onMouseEnter={() => setPanel("more")}>
              <button
                type="button"
                onClick={() =>
                  setPanel((cur) => (cur === "more" ? null : "more"))
                }
                className="flex w-full items-center gap-2 px-3 py-2.5 text-left text-[13px] text-ink hover:bg-panel2"
              >
                <span className="flex-1">{t("chat.moreModels")}</span>
                <Chevron dir="right" />
              </button>
              {panel === "more" && (
                <Submenu>
                  <div className="max-h-64 overflow-y-auto">
                    {extra.map((m) => (
                      <button
                        key={m}
                        type="button"
                        onClick={() => pickModel(m)}
                        className="flex w-full items-center gap-2 px-3 py-2 text-left text-[13px] text-ink hover:bg-panel2"
                      >
                        <span className="min-w-0 flex-1 truncate">
                          {shortModel(m)}
                        </span>
                        <Origem modelRef={m} />
                        {value === m && <Check />}
                      </button>
                    ))}
                  </div>
                </Submenu>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
