// Seletor combinado de modelo + esforço, no estilo do picker do composer:
// gatilho "Nome Alto ▾", lista de modelos e esforço / mais modelos no rodapé.

import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { getServerProps } from "../../lib/api";
import {
  EFFORT_MAX_TOKENS,
  type ChatParams,
  type EffortLevel,
} from "../../lib/types";

const EFFORTS: EffortLevel[] = ["low", "medium", "high", "extra", "max"];
const PRIMARY_LIMIT = 4;

function shortModel(name: string): string {
  return name
    .replace(/\.gguf$/i, "")
    .replace(/-UD-.*$/i, "")
    .replace(/-Q\d.*$/i, "");
}

function Check() {
  return (
    <svg
      className="h-4 w-4 shrink-0 text-sky-400"
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

function useDismiss(open: boolean, onClose: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onClose]);
  return ref;
}

function Submenu({ children }: { children: ReactNode }) {
  return (
    <div className="absolute right-full bottom-0 z-40 mr-1 min-w-48 rounded-xl border border-edge bg-panel py-1.5 shadow-[0_12px_40px_rgba(0,0,0,0.45)]">
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
  const [open, setOpen] = useState(false);
  const [panel, setPanel] = useState<"effort" | "more" | null>(null);
  // null = desconhecido (servidor fora do ar / comando indisponível): a UI
  // não mostra nada nesse caso, falha de rede aqui é silenciosa.
  const [supportsTools, setSupportsTools] = useState<boolean | null>(null);
  const ref = useDismiss(open, () => {
    setOpen(false);
    setPanel(null);
  });

  // Capacidade de ferramentas vem do chat template do modelo carregado
  // (GET /props), nunca do nome do modelo.
  useEffect(() => {
    let cancelled = false;
    setSupportsTools(null);
    if (!value) return;
    void getServerProps()
      .then((props) => {
        if (!cancelled) setSupportsTools(props.chatTemplateCaps.supportsTools);
      })
      .catch(() => {
        if (!cancelled) setSupportsTools(null);
      });
    return () => {
      cancelled = true;
    };
  }, [value]);

  const options =
    value && !models.includes(value) ? [value, ...models] : models;
  const empty = options.length === 0;
  const { primary, extra } = splitModels(options, value);

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
        title={
          empty
            ? t("chat.modelSelect")
            : [
                value,
                `${t("chat.effort.label")}: ${t(`chat.effort.${params.effort}`)}`,
                supportsTools == null
                  ? null
                  : supportsTools
                    ? t("agent.supported")
                    : t("agent.unsupported"),
              ]
                .filter(Boolean)
                .join(" · ")
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
        {!empty && (
          <span className="shrink-0 text-dim">
            {t(`chat.effort.${params.effort}`)}
          </span>
        )}
        {!empty && supportsTools === false && (
          <span
            aria-label={t("agent.unsupportedShort")}
            className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn"
          />
        )}
        <Chevron />
      </button>

      {open && (
        <div className="absolute right-0 bottom-full z-30 mb-2 w-80 rounded-xl border border-edge bg-panel py-1.5 shadow-[0_12px_40px_rgba(0,0,0,0.45)]">
          {supportsTools === false && (
            <p className="flex items-center gap-1.5 px-3 pt-1 pb-2 text-[11px] text-warn">
              <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn" />
              {t("agent.unsupportedShort")}
            </p>
          )}
          {primary.map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => pickModel(m)}
              className="flex w-full items-start gap-2 px-3 py-2.5 text-left hover:bg-panel2"
            >
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[13px] text-ink">
                  {shortModel(m)}
                </span>
                <span className="mt-0.5 block truncate text-[11px] text-dim">
                  {m}
                </span>
              </span>
              {value === m && <Check />}
            </button>
          ))}

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
